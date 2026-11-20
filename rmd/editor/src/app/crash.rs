use std::{
    backtrace::Backtrace,
    env,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    panic::{self, AssertUnwindSafe, PanicHookInfo},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::SystemTime,
};

use super::logging;
use crate::{
    session::Session,
    settings::paths::{crash_dir, recovery_dir},
};

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub(super) use windows::{unwatch_session, watch_session};

const KEEP_REPORTS: usize = 20;
const STAMP_FORMAT: &str = "%Y-%m-%d_%H-%M-%S";

static LAST_REPORT: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(super) struct CrashInfo {
    pub version: String,
    pub git_hash: Option<String>,
    pub os: String,
    pub arch: String,
    pub thread: String,
    pub message: String,
    pub location: String,
    pub backtrace: String,
    pub log_lines: Option<Vec<String>>,
    pub dump: Option<PathBuf>,
}

impl CrashInfo {
    fn new(thread: String, message: String, location: String, backtrace: String) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            git_hash: option_env!("RMD_GIT_SHORT_HASH").map(str::to_owned),
            os: env::consts::OS.to_owned(),
            arch: env::consts::ARCH.to_owned(),
            thread,
            message,
            location,
            backtrace,
            log_lines: logging::recent_lines(),
            dump: None,
        }
    }
}

pub(super) struct Recovered {
    pub map: String,
    pub path: PathBuf,
}

pub(super) fn install() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let crash = CrashInfo {
            dump: panic_dump(),
            ..gather(info)
        };
        logging::record_crash(&format!("panic: {} at {}", crash.message, crash.location));
        record(&crash);
        previous(info);
    }));

    #[cfg(target_os = "windows")]
    windows::install();
}

#[cfg(target_os = "windows")]
fn panic_dump() -> Option<PathBuf> {
    windows::dump_for_panic()
}

#[cfg(not(target_os = "windows"))]
fn panic_dump() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "windows"))]
pub(super) fn watch_session(_: *const Session) {}

#[cfg(not(target_os = "windows"))]
pub(super) fn unwatch_session() {}

fn record(info: &CrashInfo) {
    match write_report(info) {
        Ok(path) => {
            if let Ok(mut last) = LAST_REPORT.try_lock() {
                *last = Some(path);
            }
        },
        Err(error) => eprintln!("could not write the crash report: {error}"),
    }
}

pub(super) fn last_report() -> Option<PathBuf> {
    LAST_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn gather(info: &PanicHookInfo<'_>) -> CrashInfo {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".to_owned());
    let location = info
        .location()
        .map(|location| format!("{}:{}:{}", location.file(), location.line(), location.column()))
        .unwrap_or_else(|| "<unknown>".to_owned());
    let thread = thread::current().name().unwrap_or("<unnamed>").to_owned();

    CrashInfo::new(thread, message, location, Backtrace::force_capture().to_string())
}

fn format_report(info: &CrashInfo) -> String {
    let mut report = String::from("rmd crash report\n\n");
    let _ = write!(report, "version: {}", info.version);
    if let Some(hash) = &info.git_hash {
        let _ = write!(report, " ({hash})");
    }

    let _ = writeln!(report, "\nplatform: {} {}", info.os, info.arch);
    let _ = writeln!(report, "thread: {}", info.thread);
    let _ = writeln!(report, "location: {}", info.location);
    if let Some(dump) = &info.dump {
        let _ = writeln!(report, "minidump: {}", dump.display());
    }

    let _ = writeln!(report, "\nmessage:\n{}", info.message);
    let _ = writeln!(report, "\nbacktrace:\n{}", info.backtrace.trim_end());
    report.push_str("\nrecent log:\n");
    match &info.log_lines {
        Some(lines) => {
            for line in lines {
                let _ = writeln!(report, "{line}");
            }
        },
        None => report.push_str("(log unavailable)\n"),
    }

    report
}

fn stamp(now: SystemTime) -> io::Result<String> {
    let zoned = jiff::Zoned::try_from(now).map_err(io::Error::other)?;

    Ok(zoned.strftime(STAMP_FORMAT).to_string())
}

fn create_unique(dir: &Path, prefix: &str, stamp: &str, extension: &str) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(dir)?;

    for n in 1.. {
        let name = match n {
            1 => format!("{prefix}{stamp}.{extension}"),
            _ => format!("{prefix}{stamp}-{n}.{extension}"),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error),
        }
    }

    unreachable!()
}

fn write_report(info: &CrashInfo) -> io::Result<PathBuf> {
    let dir = crash_dir()?;
    let (path, mut file) = create_unique(&dir, "crash-", &stamp(SystemTime::now())?, "txt")?;
    file.write_all(format_report(info).as_bytes())?;
    let _ = prune_reports(&dir, "txt");
    let _ = prune_reports(&dir, "dmp");

    Ok(path)
}

fn prune_reports(dir: &Path, kind: &str) -> io::Result<()> {
    let mut reports = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == kind)
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("crash-"))
        })
        .collect::<Vec<_>>();
    reports.sort();

    let excess = reports.len().saturating_sub(KEEP_REPORTS);
    for path in &reports[..excess] {
        let _ = fs::remove_file(path);
    }

    Ok(())
}

pub(super) fn recover(session: &Session, root: &Path, now: SystemTime) -> (Vec<Recovered>, Vec<String>) {
    let mut recovered = Vec::new();
    let mut failures = Vec::new();
    for document in session.state.documents() {
        if !document.is_dirty() || document.is_stash() {
            continue;
        }

        let map = document
            .path
            .as_ref()
            .map_or_else(|| document.title(), |path| path.display().to_string());
        let stem = document
            .path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map_or_else(|| "untitled".to_owned(), |stem| stem.to_string_lossy().into_owned());

        let attempt = panic::catch_unwind(AssertUnwindSafe(|| -> io::Result<PathBuf> {
            let contents = document.export();
            let (path, mut file) = create_unique(&root.join(&stem), "", &stamp(now)?, "dmm")?;
            file.write_all(contents.as_bytes())?;

            Ok(path)
        }));
        match attempt {
            Ok(Ok(path)) => recovered.push(Recovered { map, path }),
            Ok(Err(error)) => failures.push(format!("{map}: {error}")),
            Err(_) => failures.push(format!("{map}: exporting the map panicked")),
        }
    }

    (recovered, failures)
}

pub(super) fn recovery_summary(recovered: &[Recovered], failures: &[String]) -> String {
    let mut summary = String::new();
    if recovered.is_empty() && failures.is_empty() {
        summary.push_str("There were no unsaved changes.");
    }

    if !recovered.is_empty() {
        summary.push_str("Unsaved maps were recovered to:");
        for item in recovered {
            let _ = write!(summary, "\n{} -> {}", item.map, item.path.display());
        }
    }

    if !failures.is_empty() {
        if !summary.is_empty() {
            summary.push_str("\n\n");
        }

        summary.push_str("Could not recover (sorry):");
        for failure in failures {
            let _ = write!(summary, "\n{failure}");
        }
    }

    summary
}

pub(super) fn handle_main_thread_crash(session: &Session) -> String {
    let report = last_report();
    let (recovered, failures) = match recovery_dir() {
        Ok(root) => recover(session, &root, SystemTime::now()),
        Err(error) => (Vec::new(), vec![format!("no recovery folder: {error}")]),
    };
    let summary = recovery_summary(&recovered, &failures);

    if let Some(path) = &report
        && let Ok(mut file) = OpenOptions::new().append(true).open(path)
    {
        let _ = write!(file, "\nrecovered maps:\n{summary}\n");
    }

    crash_message(report.as_deref(), &summary)
}

fn crash_message(report: Option<&Path>, summary: &str) -> String {
    let mut message = String::from("rmd hit an internal error and has to close.\n\n");
    match report {
        Some(path) => {
            let _ = write!(message, "The crash report was written to:\n{}\n\n", path.display());
        },
        None => message.push_str("The crash report could not be written.\n\n"),
    }

    message.push_str(summary);

    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmm::{Map, Size};

    use crate::{document::MapDocument, testing::TempDir};

    fn sample() -> CrashInfo {
        CrashInfo {
            version: "1.2.3".to_owned(),
            git_hash: Some("abc1234".to_owned()),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            thread: "main".to_owned(),
            message: "boom".to_owned(),
            location: "src/lib.rs:1:2".to_owned(),
            backtrace: "   0: frame_zero\n   1: frame_one".to_owned(),
            log_lines: Some(vec!["[INFO] first".to_owned(), "[ERROR] second".to_owned()]),
            dump: Some(PathBuf::from("crash-1.dmp")),
        }
    }

    #[test]
    fn the_report_lists_every_field_in_order() {
        let report = format_report(&sample());

        let positions = [
            "1.2.3 (abc1234)",
            "linux x86_64",
            "thread: main",
            "src/lib.rs:1:2",
            "boom",
            "frame_zero",
            "frame_one",
            "[INFO] first",
            "[ERROR] second",
        ]
        .map(|needle| report.find(needle).unwrap_or_else(|| panic!("missing {needle}")));
        assert!(positions.is_sorted());
    }

    #[test]
    fn the_report_says_when_the_log_is_unavailable() {
        let mut info = sample();
        info.log_lines = None;

        assert!(format_report(&info).contains("(log unavailable)"));
    }

    #[test]
    fn unique_files_do_not_overwrite_each_other() {
        let dir = TempDir::new("crash-unique");

        let (first, _) = create_unique(&dir.join("a"), "", "stamp", "dmm").unwrap();
        let (second, _) = create_unique(&dir.join("a"), "", "stamp", "dmm").unwrap();
        let (third, _) = create_unique(&dir.join("a"), "", "stamp", "dmm").unwrap();

        assert_eq!(first.file_name().unwrap(), "stamp.dmm");
        assert_eq!(second.file_name().unwrap(), "stamp-2.dmm");
        assert_eq!(third.file_name().unwrap(), "stamp-3.dmm");
    }

    #[test]
    fn only_the_newest_reports_are_kept() {
        let dir = TempDir::new("crash-prune");
        for n in 0..KEEP_REPORTS + 3 {
            fs::write(dir.join(format!("crash-2026-01-01_00-00-{n:02}.txt")), "").unwrap();
        }
        fs::write(dir.join("other.txt"), "").unwrap();

        prune_reports(&dir, "txt").unwrap();

        let mut names = fs::read_dir(&*dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names.len(), KEEP_REPORTS + 1);
        assert!(names.contains(&"other.txt".to_owned()));
        assert!(!names.contains(&"crash-2026-01-01_00-00-00.txt".to_owned()));
        assert!(names.contains(&format!("crash-2026-01-01_00-00-{:02}.txt", KEEP_REPORTS + 2)));
    }

    fn dirty_session(dir: &Path) -> (Session, PathBuf, PathBuf) {
        let map = || Map::new(Size { x: 2, y: 2, z: 1 });
        let dirty = dir.join("dirty.dmm");
        let clean = dir.join("clean.dmm");
        fs::write(&dirty, "original dirty").unwrap();
        fs::write(&clean, "original clean").unwrap();

        let mut session = Session::new();
        session
            .state
            .open_document(MapDocument::open_modified(&dirty, map(), 1));
        session.state.open_document(MapDocument::open(&clean, map(), 1));
        let mut untitled = MapDocument::new(map(), 1);
        untitled.mark_unsaved();
        session.state.open_document(untitled);

        (session, dirty, clean)
    }

    #[test]
    fn recovery_writes_only_dirty_maps_and_leaves_the_originals_alone() {
        let dir = TempDir::new("crash-recover");
        let (session, dirty, clean) = dirty_session(&dir);
        let root = dir.join("recovery");

        let (recovered, failures) = recover(&session, &root, SystemTime::now());

        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].path.parent().unwrap(), root.join("dirty"));
        assert_eq!(recovered[1].path.parent().unwrap(), root.join("untitled"));
        let expected = session.state.documents()[0].export();
        assert_eq!(fs::read_to_string(&recovered[0].path).unwrap(), expected);
        assert_eq!(fs::read(&dirty).unwrap(), b"original dirty");
        assert_eq!(fs::read(&clean).unwrap(), b"original clean");
        assert!(session.state.documents()[0].is_dirty());
        assert!(!root.join("clean").exists());
    }

    #[test]
    fn recovering_twice_in_one_second_keeps_both_copies() {
        let dir = TempDir::new("crash-recover-twice");
        let (session, _, _) = dirty_session(&dir);
        let root = dir.join("recovery");
        let now = SystemTime::now();

        let (first, _) = recover(&session, &root, now);
        let (second, _) = recover(&session, &root, now);

        assert_ne!(first[0].path, second[0].path);
        assert!(first[0].path.exists() && second[0].path.exists());
    }

    #[test]
    fn the_message_names_the_report_and_the_recovered_files() {
        let recovered = [Recovered {
            map: "a.dmm".to_owned(),
            path: PathBuf::from("/r/a/1.dmm"),
        }];

        let summary = recovery_summary(&recovered, &["b.dmm: disk full".to_owned()]);

        assert!(summary.contains("/r/a/1.dmm"));
        assert!(summary.contains("b.dmm: disk full"));
        assert!(recovery_summary(&[], &[]).contains("no unsaved changes"));
    }
}
