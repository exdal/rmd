use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    emit_git_rerun_paths(&manifest_dir);

    let Some(hash) = git_output(&manifest_dir, &["rev-parse", "HEAD"]) else {
        return;
    };
    let short_hash = hash.chars().take(7).collect::<String>();
    println!("cargo:rustc-env=RMD_GIT_SHORT_HASH={short_hash}");

    let Some(origin) = git_output(&manifest_dir, &["remote", "get-url", "origin"]) else {
        return;
    };
    let Some(repository_url) = normalize_repository_url(&origin) else {
        return;
    };
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");
    println!("cargo:rustc-env=RMD_VERSION_URL={repository_url}/releases/tag/v{version}");
    println!("cargo:rustc-env=RMD_COMMIT_URL={repository_url}/commit/{hash}");
    println!("cargo:rustc-env=RMD_REPOSITORY_URL={repository_url}");
}

fn git_output(manifest_dir: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.trim();
    (!output.is_empty()).then(|| output.to_owned())
}

fn emit_git_rerun_paths(manifest_dir: &Path) {
    let Some(git_dir) = git_output(manifest_dir, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) else {
        return;
    };
    let common_dir = git_output(manifest_dir, &["rev-parse", "--git-common-dir"])
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                manifest_dir.join(path)
            }
        })
        .unwrap_or_else(|| git_dir.clone());

    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!("cargo:rerun-if-changed={}", common_dir.join("config").display());
    println!("cargo:rerun-if-changed={}", common_dir.join("packed-refs").display());

    if let Ok(contents) = fs::read_to_string(&head)
        && let Some(reference) = contents.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed={}", common_dir.join(reference).display());
    }
}

fn normalize_repository_url(origin: &str) -> Option<String> {
    let origin = origin.trim().trim_end_matches('/').trim_end_matches(".git");
    if origin.starts_with("https://") || origin.starts_with("http://") {
        return Some(origin.to_owned());
    }
    if let Some(origin) = origin.strip_prefix("git://") {
        return Some(format!("https://{origin}"));
    }
    if let Some(origin) = origin.strip_prefix("ssh://") {
        let origin = origin.rsplit_once('@').map_or(origin, |(_, path)| path);
        return Some(format!("https://{origin}"));
    }

    let (_, origin) = origin.rsplit_once('@')?;
    let (host, path) = origin.split_once(':')?;
    Some(format!("https://{host}/{path}"))
}
