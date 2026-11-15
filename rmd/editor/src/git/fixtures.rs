use std::{fs, path::Path, process::Command};

use crate::testing::TempDir;

pub(crate) fn git(dir: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=rmd",
            "-c",
            "user.email=rmd@example.com",
            "-c",
            "core.autocrlf=false",
        ])
        .args(arguments)
        .current_dir(dir)
        .output()
        .ok()?;

    Some(String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
}

pub(crate) fn git_available() -> bool {
    git(Path::new("."), &["--version"]).is_some()
}

pub(crate) fn temp_repo(name: &str) -> TempDir {
    let dir = TempDir::new(&format!("git-{name}"));
    git(&dir, &["init", "-q", "-b", "main"]);

    dir
}

pub(crate) fn commit(dir: &Path, file: &str, contents: &str, message: &str) {
    fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", message]);
}
