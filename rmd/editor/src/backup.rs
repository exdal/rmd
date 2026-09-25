use std::{
    fs,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const KEEP_FOR: Duration = Duration::from_secs(3 * 24 * 60 * 60);

pub fn back_up(root: &Path, map: &Path, source: &str, now: SystemTime) -> io::Result<()> {
    let Some(name) = map.file_stem() else {
        return Ok(());
    };
    prune(root, now - KEEP_FOR)?;

    let dir = root.join(name);
    if newest_backup(&dir)?.is_some_and(|newest| fs::read(newest).is_ok_and(|bytes| bytes == source.as_bytes())) {
        return Ok(());
    }

    let stamp = jiff::Zoned::try_from(now).map_err(io::Error::other)?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{}.dmm", stamp.strftime("%Y-%m-%d_%H-%M-%S"))), source)
}

fn backups(dir: &Path) -> io::Result<Vec<(PathBuf, SystemTime)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut backups = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "dmm") && entry.file_type()?.is_file() {
            backups.push((path, entry.metadata()?.modified()?));
        }
    }

    Ok(backups)
}

fn newest_backup(dir: &Path) -> io::Result<Option<PathBuf>> {
    Ok(backups(dir)?
        .into_iter()
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path))
}

fn prune(root: &Path, cutoff: SystemTime) -> io::Result<()> {
    let maps = match fs::read_dir(root) {
        Ok(maps) => maps,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    for map in maps {
        let dir = map?.path();
        if !dir.is_dir() {
            continue;
        }
        for (path, modified) in backups(&dir)? {
            if modified < cutoff {
                fs::remove_file(path)?;
            }
        }
        // fails while backups remain, which is the point
        let _ = fs::remove_dir(&dir);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rmd-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<_> = backups(dir).unwrap().into_iter().map(|(path, _)| path).collect();
        files.sort();
        files
    }

    #[test]
    fn opening_a_map_backs_it_up_once_per_change() {
        let root = scratch("changes");
        let map = Path::new("maps/station.dmm");
        let now = SystemTime::now();

        back_up(&root, map, "first", now).unwrap();
        back_up(&root, map, "first", now + Duration::from_secs(60)).unwrap();
        assert_eq!(files(&root.join("station")).len(), 1);

        back_up(&root, map, "second", now + Duration::from_secs(120)).unwrap();
        let saved = files(&root.join("station"));
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().any(|path| fs::read_to_string(path).unwrap() == "second"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn backups_older_than_three_days_are_deleted_for_every_map() {
        let root = scratch("prune");
        let now = SystemTime::now();
        let stale = root.join("old/2020-01-01_00-00-00.dmm");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, "old").unwrap();
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(now - KEEP_FOR - Duration::from_secs(1))
            .unwrap();

        back_up(&root, Path::new("station.dmm"), "fresh", now).unwrap();

        assert!(!stale.exists());
        assert!(!root.join("old").exists());
        assert_eq!(files(&root.join("station")).len(), 1);

        fs::remove_dir_all(&root).unwrap();
    }
}
