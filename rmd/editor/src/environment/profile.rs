use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use base::location::FileId;
use objtree::{ObjectTree, TypeId};

use super::{
    Profiles,
    compile::{BakeOptions, ForcedSources},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundledProfile {
    Beestation,
    Cmss13,
    Goonstation,
    Monkestation,
    Monkespaw,
    Novasector,
    Oculis,
    SecondCity,
    Tgmc,
    Tgstation,
    Vanderlin,
}

impl BundledProfile {
    pub const ALL: [Self; 11] = [
        Self::Beestation,
        Self::Cmss13,
        Self::Goonstation,
        Self::Monkestation,
        Self::Monkespaw,
        Self::Novasector,
        Self::Oculis,
        Self::SecondCity,
        Self::Tgmc,
        Self::Tgstation,
        Self::Vanderlin,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Beestation => "BeeStation",
            Self::Cmss13 => "CMSS13",
            Self::Goonstation => "Goonstation",
            Self::Monkestation => "Monkestation",
            Self::Monkespaw => "Monke’s Paw",
            Self::Novasector => "Nova Sector",
            Self::Oculis => "Oculis Station",
            Self::SecondCity => "SecondCity",
            Self::Tgmc => "TGMC",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "Vanderlin",
        }
    }

    pub const fn stem(self) -> &'static str {
        match self {
            Self::Beestation => "beestation",
            Self::Cmss13 => "cmss13",
            Self::Goonstation => "goonstation",
            Self::Monkestation => "monkestation",
            Self::Monkespaw => "monkespaw",
            Self::Novasector => "novasector",
            Self::Oculis => "oculis",
            Self::SecondCity => "secondcity",
            Self::Tgmc => "tgmc",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "vanderlin",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|profile| profile.stem() == name)
    }

    pub(super) const fn source_name(self) -> &'static str {
        match self {
            Self::Beestation => "<bundled-profile-beestation.dm>",
            Self::Cmss13 => "<bundled-profile-cmss13.dm>",
            Self::Goonstation => "<bundled-profile-goonstation.dm>",
            Self::Monkestation => "<bundled-profile-monkestation.dm>",
            Self::Monkespaw => "<bundled-profile-monkespaw.dm>",
            Self::Novasector => "<bundled-profile-novasector.dm>",
            Self::Oculis => "<bundled-profile-oculis.dm>",
            Self::SecondCity => "<bundled-profile-secondcity.dm>",
            Self::Tgmc => "<bundled-profile-tgmc.dm>",
            Self::Tgstation => "<bundled-profile-tgstation.dm>",
            Self::Vanderlin => "<bundled-profile-vanderlin.dm>",
        }
    }

    pub(super) const fn source(self) -> &'static str {
        match self {
            Self::Beestation => include_str!("../../profiles/beestation.dm"),
            Self::Cmss13 => include_str!("../../profiles/cmss13.dm"),
            Self::Goonstation => include_str!("../../profiles/goonstation.dm"),
            Self::Monkestation => include_str!("../../profiles/monkestation.dm"),
            Self::Monkespaw => include_str!("../../profiles/monkespaw.dm"),
            Self::Novasector => include_str!("../../profiles/novasector.dm"),
            Self::Oculis => include_str!("../../profiles/oculis.dm"),
            Self::SecondCity => include_str!("../../profiles/secondcity.dm"),
            Self::Tgmc => include_str!("../../profiles/tgmc.dm"),
            Self::Tgstation => include_str!("../../profiles/tgstation.dm"),
            Self::Vanderlin => include_str!("../../profiles/vanderlin.dm"),
        }
    }

    pub(super) const fn defines(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Goonstation => Some((
                "<bundled-profile-goonstation-defines.dm>",
                "#define USE_PERSPECTIVE_EDITOR_WALLS\n",
            )),
            Self::Beestation => Some((
                "<bundled-profile-beestation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            // COCK AND BALL TORTURE
            Self::Tgstation => Some((
                "<bundled-profile-tgstation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::SecondCity => Some((
                "<bundled-profile-secondcity-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Monkestation => Some((
                "<bundled-profile-monkestation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Monkespaw => Some((
                "<bundled-profile-monkespaw-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Novasector => Some((
                "<bundled-profile-novasector-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Oculis => Some((
                "<bundled-profile-oculis-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Cmss13 | Self::Tgmc | Self::Vanderlin => None,
        }
    }
}

pub(super) const PROFILE_EXTENSION: &str = ".dm";
pub(super) const DEFINES_EXTENSION: &str = ".defines.dm";

pub fn write_missing_profiles(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for profile in BundledProfile::ALL {
        write_new(
            &dir.join(format!("{}{PROFILE_EXTENSION}", profile.stem())),
            profile.source(),
        )?;
        if let Some((_, defines)) = profile.defines() {
            write_new(&dir.join(format!("{}{DEFINES_EXTENSION}", profile.stem())), defines)?;
        }
    }

    Ok(())
}

fn write_new(path: &Path, contents: &str) -> io::Result<()> {
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(contents.as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn scan_profiles(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::warn!("reading profiles from {}: {error}", dir.display());

            return BundledProfile::ALL.map(|profile| profile.stem().to_owned()).to_vec();
        },
    };

    let mut names = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|file| !file.ends_with(DEFINES_EXTENSION))
        .filter_map(|file| file.strip_suffix(PROFILE_EXTENSION).map(str::to_owned))
        .collect::<Vec<_>>();
    names.sort();

    names
}

pub fn profile_label(name: &str) -> &str {
    match BundledProfile::from_name(name) {
        Some(profile) => profile.label(),
        None => name,
    }
}

#[derive(Default)]
pub(super) struct ProfileSelection {
    pub(super) chosen: Option<(TypeId, Profiles)>,
    pub(super) error: Option<vm::bake::ProfileError>,
}

pub(super) fn select_profile(
    tree: &ObjectTree, files: &[PathBuf], options: &BakeOptions, forced: &ForcedSources,
) -> ProfileSelection {
    let path = |id| {
        tree.get(id)
            .map(|declaration| declaration.path.to_string())
            .unwrap_or_default()
    };
    let (chosen, error) = match options.forced_profile {
        Some(_) => match forced
            .profile
            .as_ref()
            .and_then(|profile| files.iter().position(|file| file == profile.path()))
            .map_or(Err(vm::bake::ProfileError::Missing), |file| {
                vm::bake::profile_in_file(tree, FileId(file as u32))
            }) {
            Ok(active) => {
                let active_path = path(active);
                let profiles = Profiles {
                    available: vec![active_path.clone()],
                    default: active_path.clone(),
                    active: active_path,
                };

                (Some((active, profiles)), None)
            },
            Err(error) => (None, Some(error)),
        },
        None => match vm::bake::profile_catalog(tree) {
            Ok(catalog) => {
                let active = catalog.select(tree, options.profile.as_ref());
                let profiles = Profiles {
                    available: catalog.profiles.iter().copied().map(path).collect::<Vec<_>>(),
                    default: path(catalog.default),
                    active: path(active),
                };

                (Some((active, profiles)), None)
            },
            Err(vm::bake::ProfileError::Missing) => (None, None),
            Err(error) => (None, Some(error)),
        },
    };

    ProfileSelection { chosen, error }
}
