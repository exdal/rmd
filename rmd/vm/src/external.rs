use base::file_path;
use libloading::Library;
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, PoisonError, RwLock},
};

use crate::FaultKind;

type Symbol = unsafe extern "C" fn(c_int, *const *const c_char) -> *const c_char;

static LIBRARIES: LazyLock<RwLock<HashMap<PathBuf, &'static Library>>> = LazyLock::new(RwLock::default);

static CALLS: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExternalAccess {
    #[default]
    Blocked,
    Trusted,
}

#[derive(Debug)]
pub struct ExternalFunction {
    pub library: PathBuf,
    pub name: Box<str>,
    symbol: Symbol,
}

impl ExternalFunction {
    pub(crate) fn load(root: &Path, library: &str, function: &str) -> Result<Self, FaultKind> {
        if function.contains(':') {
            // I refuse to support byondapi
            return Err(FaultKind::Unsupported(format!("byondapi call_ext {function}")));
        }

        let name = function.trim_start_matches('@');
        let path = locate(root, library)?;
        let library = open(&path)?;

        let symbol_name =
            CString::new(name).map_err(|_| FaultKind::External(format!("invalid function name {name:?}")))?;

        // hmm if the editor starts crashing this is the:
        // SUSPECT 1
        let symbol = unsafe { library.get::<Symbol>(symbol_name.as_bytes_with_nul()) }
            .map(|symbol| *symbol)
            .map_err(|error| FaultKind::External(format!("{name} in {}: {error}", path.display())))?;

        Ok(Self {
            library: path,
            name: name.into(),
            symbol,
        })
    }

    pub(crate) fn call(&self, root: &Path, args: &[&str]) -> Result<Option<String>, FaultKind> {
        let args = args
            .iter()
            .map(|arg| CString::new(arg.split('\0').next().unwrap_or_default()).unwrap_or_default())
            .collect::<Vec<_>>();
        let pointers = args.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();

        let _call = CALLS.lock().unwrap_or_else(PoisonError::into_inner);
        let _directory = WorkingDirectory::enter(root)?;

        // hopefully that symbol is not dangling
        // SUSPECT 2
        let result = unsafe {
            let result = (self.symbol)(pointers.len() as c_int, pointers.as_ptr());
            (!result.is_null()).then(|| CStr::from_ptr(result).to_string_lossy().into_owned())
        };

        Ok(result)
    }
}

/// Dylibs such as rust_g resolve relative paths against it.
struct WorkingDirectory {
    previous: Option<PathBuf>,
}

impl WorkingDirectory {
    fn enter(root: &Path) -> Result<Self, FaultKind> {
        let previous = std::env::current_dir().ok();
        std::env::set_current_dir(root).map_err(|error| FaultKind::External(format!("{}: {error}", root.display())))?;

        Ok(Self { previous })
    }
}

impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            let _ = std::env::set_current_dir(previous);
        }
    }
}

impl PartialEq for ExternalFunction {
    fn eq(&self, other: &Self) -> bool {
        self.library == other.library && self.name == other.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Windows,
    Linux,
    MacOs,
}

impl Platform {
    const HOST: Self = if cfg!(windows) {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Linux
    };

    fn extension(self) -> &'static str {
        match self {
            Self::Windows => "dll",
            Self::Linux => "so",
            Self::MacOs => "dylib",
        }
    }
}

fn locate(root: &Path, library: &str) -> Result<PathBuf, FaultKind> {
    let names = candidates(library, Platform::HOST);
    let byond_bin = std::env::var_os("HOME")
        .filter(|_| Platform::HOST == Platform::Linux)
        .map(|home| PathBuf::from(home).join(".byond/bin"));
    let tried = [Some(root), byond_bin.as_deref()]
        .into_iter()
        .flatten()
        .flat_map(|directory| names.iter().filter_map(|name| file_path::within(directory, name)))
        .collect::<Vec<_>>();

    if let Some(path) = tried.iter().find(|path| path.is_file()) {
        return Ok(path.clone());
    }

    let tried = tried
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(FaultKind::External(format!("{library} not found (tried {tried})")))
}

fn candidates(library: &str, platform: Platform) -> Vec<String> {
    let library = file_path::forward_slashes(library);
    let (directory, file) = match library.rsplit_once('/') {
        Some((directory, file)) => (format!("{directory}/"), file),
        None => (String::new(), library.as_ref()),
    };
    let (stem, extension) = match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (file, None),
    };

    let prefixes = match platform {
        Platform::Linux | Platform::MacOs if !stem.starts_with("lib") => &["", "lib"][..],
        _ => &[""][..],
    };

    let mut names = Vec::new();
    for suffix in ["64", ""] {
        for prefix in prefixes {
            let base = format!("{directory}{prefix}{stem}{suffix}");
            match extension {
                Some(extension) => names.push(format!("{base}.{extension}")),
                None => {
                    names.push(format!("{base}.{}", platform.extension()));
                    names.push(base);
                },
            }
        }
    }

    names
}

fn open(path: &Path) -> Result<&'static Library, FaultKind> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(library) = LIBRARIES.read().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return Ok(library);
    }

    // another thread may have loaded it between the read and the write lock
    let mut libraries = LIBRARIES.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(library) = libraries.get(&key) {
        return Ok(library);
    }

    let library = unsafe { Library::new(&key) }.map_err(|error| {
        let message = match foreign_binary(&key) {
            Some(kind) => format!(
                "{} is {kind}. This editor needs a 64-bit {} build",
                key.display(),
                host_name()
            ),
            None => format!("{}: {error}", key.display()),
        };
        FaultKind::External(message)
    })?;
    let library: &'static Library = Box::leak(Box::new(library));
    libraries.insert(key, library);

    Ok(library)
}

fn host_name() -> &'static str {
    match Platform::HOST {
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        Platform::MacOs => "macOS",
    }
}

// for the sake of UX, we can also parse the binary header
fn foreign_binary(path: &Path) -> Option<&'static str> {
    let mut header = Vec::new();
    File::open(path).ok()?.take(4096).read_to_end(&mut header).ok()?;

    match header.get(..5)? {
        [b'M', b'Z', ..] => {
            let offset = u32::from_le_bytes(header.get(0x3c..0x40)?.try_into().ok()?) as usize;
            let machine = u16::from_le_bytes(header.get(offset + 4..offset + 6)?.try_into().ok()?);
            match (machine, Platform::HOST) {
                (0x14c, _) => Some("a 32-bit Windows library"),
                (_, Platform::Linux | Platform::MacOs) => Some("a Windows library"),
                _ => None,
            }
        },
        [0x7f, b'E', b'L', b'F', class] => match (class, Platform::HOST) {
            (1, _) => Some("a 32-bit Linux library"),
            (_, Platform::Windows | Platform::MacOs) => Some("a Linux library"),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Platform, candidates, foreign_binary};

    #[test]
    fn candidates_try_the_64_bit_name_first() {
        let cases = [
            (
                "rust_g",
                Platform::Windows,
                &["rust_g64.dll", "rust_g64", "rust_g.dll", "rust_g"][..],
            ),
            ("test.DLL", Platform::Windows, &["test64.DLL", "test.DLL"][..]),
            (
                "rust_g",
                Platform::Linux,
                &[
                    "rust_g64.so",
                    "rust_g64",
                    "librust_g64.so",
                    "librust_g64",
                    "rust_g.so",
                    "rust_g",
                    "librust_g.so",
                    "librust_g",
                ][..],
            ),
            (
                "merge",
                Platform::MacOs,
                &[
                    "merge64.dylib",
                    "merge64",
                    "libmerge64.dylib",
                    "libmerge64",
                    "merge.dylib",
                    "merge",
                    "libmerge.dylib",
                    "libmerge",
                ][..],
            ),
            (
                "./librust_g.so",
                Platform::Linux,
                &["./librust_g64.so", "./librust_g.so"][..],
            ),
            (
                "libs\\auxtools.dll",
                Platform::Windows,
                &["libs/auxtools64.dll", "libs/auxtools.dll"][..],
            ),
        ];

        for (library, platform, expected) in cases {
            assert_eq!(candidates(library, platform), expected, "{library} on {platform:?}");
        }
    }

    #[test]
    fn a_32_bit_windows_library_is_named_as_such() {
        let mut header = vec![0; 0x84];
        header[..2].copy_from_slice(b"MZ");
        header[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        header[0x80..0x84].copy_from_slice(b"PE\0\0");
        header.extend_from_slice(&0x14cu16.to_le_bytes());
        let path = std::env::temp_dir().join(format!("rmd-external-pe32-{}.dll", std::process::id()));
        std::fs::write(&path, header).expect("write header");

        assert_eq!(foreign_binary(&path), Some("a 32-bit Windows library"));
        let _ = std::fs::remove_file(path);
    }
}
