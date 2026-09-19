use std::{
    fs::File,
    io::{self, Write},
    sync::{Mutex, OnceLock},
};
#[cfg(any(target_os = "windows", test))]
use std::{
    fs::{self, OpenOptions},
    path::Path,
};

use log::{Level, LevelFilter, Log, Metadata, Record};

#[cfg(target_os = "windows")]
use crate::settings;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

pub(crate) fn init() {
    #[cfg(target_os = "windows")]
    let (file, error) = match settings::log_path().and_then(|path| open_log_file(&path)) {
        Ok(file) => (Some(file), None),
        Err(error) => (None, Some(error)),
    };
    #[cfg(not(target_os = "windows"))]
    let (file, error) = (None, None::<io::Error>);

    let mirror_to_stderr = !cfg!(target_os = "windows") || cfg!(debug_assertions) || file.is_none();
    if LOGGER
        .set(FileLogger {
            file: file.map(Mutex::new),
            mirror_to_stderr,
        })
        .is_err()
    {
        return;
    }

    let Some(logger) = LOGGER.get() else {
        return;
    };
    if log::set_logger(logger).is_err() {
        return;
    }
    log::set_max_level(LevelFilter::Info);

    if let Some(error) = error {
        log::error!("could not initialize latest.log: {error}");
    }

    #[cfg(target_os = "windows")]
    install_panic_hook();
}

struct FileLogger {
    file: Option<Mutex<File>>,
    mirror_to_stderr: bool,
}

impl FileLogger {
    fn write(&self, level: Level, arguments: std::fmt::Arguments<'_>, mirror_to_stderr: bool) {
        if let Some(file) = &self.file {
            let mut file = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = writeln!(file, "[{level}] {arguments}");
            let _ = file.flush();
        }

        if mirror_to_stderr {
            eprintln!("[{level}] {arguments}");
        }
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool { metadata.level() <= Level::Info }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            self.write(record.level(), *record.args(), self.mirror_to_stderr);
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file {
            let mut file = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = file.flush();
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    OpenOptions::new().create(true).write(true).truncate(true).open(path)
}

#[cfg(target_os = "windows")]
fn install_panic_hook() {
    #[cfg(debug_assertions)]
    let default_hook = std::panic::take_hook();
    #[cfg(not(debug_assertions))]
    let _ = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        if let Some(logger) = LOGGER.get() {
            logger.write(Level::Error, format_args!("panic: {info}"), false);
        }

        #[cfg(debug_assertions)]
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_the_log_replaces_the_previous_run_and_writes_new_records() {
        let directory = std::env::temp_dir().join(format!("rmde-log-test-{}", std::process::id()));
        let path = directory.join("latest.log");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "previous run\n").unwrap();

        let logger = FileLogger {
            file: Some(Mutex::new(open_log_file(&path).unwrap())),
            mirror_to_stderr: false,
        };
        logger.write(Level::Warn, format_args!("new warning"), false);

        assert_eq!(fs::read_to_string(&path).unwrap(), "[WARN] new warning\n");

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
