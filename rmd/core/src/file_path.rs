use std::{
    borrow::Cow,
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
}
