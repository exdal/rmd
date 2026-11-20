use std::{
    collections::VecDeque,
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
use crate::settings::paths;

const RECENT_LINES: usize = 200;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

pub(crate) fn init() {
    #[cfg(target_os = "windows")]
    let (file, error) = match paths::log_path().and_then(|path| open_log_file(&path)) {
        Ok(file) => (Some(file), None),
        Err(error) => (None, Some(error)),
    };
    #[cfg(not(target_os = "windows"))]
    let (file, error) = (None, None::<io::Error>);

    let mirror_to_stderr = !cfg!(target_os = "windows") || cfg!(debug_assertions) || file.is_none();
    if LOGGER
        .set(FileLogger {
            file: file.map(Mutex::new),
            recent: Mutex::default(),
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
}

struct FileLogger {
    file: Option<Mutex<File>>,
    recent: Mutex<VecDeque<String>>,
    mirror_to_stderr: bool,
}

impl FileLogger {
    fn write(&self, level: Level, arguments: std::fmt::Arguments<'_>, mirror_to_stderr: bool) {
        self.remember(format!("[{level}] {arguments}"));

        if let Some(file) = &self.file {
            let mut file = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = writeln!(file, "[{level}] {arguments}");
            let _ = file.flush();
        }

        if mirror_to_stderr {
            eprintln!("[{level}] {arguments}");
        }
    }

    fn try_write(&self, line: &str) {
        if let Ok(mut recent) = self.recent.try_lock() {
            if recent.len() == RECENT_LINES {
                recent.pop_front();
            }

            recent.push_back(line.to_owned());
        }

        if let Some(Ok(mut file)) = self.file.as_ref().map(Mutex::try_lock) {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }

    fn remember(&self, line: String) {
        let mut recent = self.recent.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if recent.len() == RECENT_LINES {
            recent.pop_front();
        }

        recent.push_back(line);
    }

    fn try_recent(&self) -> Option<Vec<String>> {
        let recent = self.recent.try_lock().ok()?;

        Some(recent.iter().cloned().collect())
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info
    }

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

pub(super) fn recent_lines() -> Option<Vec<String>> {
    LOGGER.get()?.try_recent()
}

pub(super) fn record_crash(message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.try_write(&format!("[ERROR] {message}"));
    }
}

#[cfg(any(target_os = "windows", test))]
fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    OpenOptions::new().create(true).write(true).truncate(true).open(path)
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
            recent: Mutex::default(),
            mirror_to_stderr: false,
        };
        logger.write(Level::Warn, format_args!("new warning"), false);

        assert_eq!(fs::read_to_string(&path).unwrap(), "[WARN] new warning\n");

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn the_recent_ring_keeps_only_the_last_lines() {
        let logger = FileLogger {
            file: None,
            recent: Mutex::default(),
            mirror_to_stderr: false,
        };
        for n in 0..RECENT_LINES + 5 {
            logger.write(Level::Info, format_args!("line {n}"), false);
        }

        let lines = logger.try_recent().unwrap();
        assert_eq!(lines.len(), RECENT_LINES);
        assert_eq!(lines[0], "[INFO] line 5");
        assert_eq!(lines[RECENT_LINES - 1], format!("[INFO] line {}", RECENT_LINES + 4));
    }
}
