use std::{
    borrow::Cow,
    fs,
    path::{Component, Path, PathBuf},
};

pub fn forward_slashes(name: &str) -> Cow<'_, str> {
    if name.contains('\\') {
        Cow::Owned(name.replace('\\', "/"))
    } else {
        Cow::Borrowed(name)
    }
}

pub fn within(root: &Path, name: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let depth = path.components().count();
    for component in Path::new(forward_slashes(name).as_ref()).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {},
            Component::ParentDir if path.components().count() > depth => {
                path.pop();
            },
            _ => return None,
        }
    }

    Some(path)
}

pub fn relative(root: &Path, file: &Path) -> Option<String> {
    let root = fs::canonicalize(root).ok()?;
    let file = fs::canonicalize(file).ok()?;
    let parts = file
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;

    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backslashes_become_forward_slashes() {
        assert_eq!(forward_slashes(r"icons\obj\doors.dmi"), "icons/obj/doors.dmi");
        assert!(matches!(forward_slashes("icons/obj/doors.dmi"), Cow::Borrowed(_)));
    }

    #[test]
    fn within_stays_under_the_root() {
        let root = Path::new("/tg");
        assert_eq!(within(root, r"_maps\a.dmm"), Some(PathBuf::from("/tg/_maps/a.dmm")));
        assert_eq!(within(root, "_maps/../b.dmm"), Some(PathBuf::from("/tg/b.dmm")));
        assert_eq!(within(root, "../outside.dmm"), None);
        assert_eq!(within(root, "/etc/passwd"), None);
    }

    #[test]
    fn relative_paths_use_forward_slashes_and_stay_inside_the_root() {
        let dir = std::env::temp_dir().join(format!("rmd-relative-{}", std::process::id()));
        let base = dir.join("base");
        fs::create_dir_all(base.join("sub")).unwrap();
        fs::write(base.join("sub/name.dmm"), "").unwrap();
        fs::write(dir.join("outside.dmm"), "").unwrap();

        assert_eq!(
            relative(&base, &base.join("sub/name.dmm")).as_deref(),
            Some("sub/name.dmm")
        );
        assert_eq!(relative(&base, &dir.join("outside.dmm")), None);
        assert_eq!(relative(&base, &base), None);
        assert_eq!(relative(&base, &base.join("missing.dmm")), None);

        fs::remove_dir_all(&dir).ok();
    }
}
