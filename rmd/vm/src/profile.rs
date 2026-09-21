use core::{
    path::TreePath,
    types::{Identifier, ProcId},
};

use objtree::{ObjectTree, ProcDecl, TypeId};

pub const BASE_PATH: &str = "/datum/demir";
pub const DEFAULT_VARIABLE: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ProfileHook {
    New,
    Prepare,
    Connections,
    Highlights,
    Light,
    Bake,
    Ui,
}

impl ProfileHook {
    pub const ALL: [Self; 7] = [
        Self::New,
        Self::Prepare,
        Self::Connections,
        Self::Highlights,
        Self::Light,
        Self::Bake,
        Self::Ui,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Prepare => "prepare",
            Self::Connections => "connections",
            Self::Highlights => "highlights",
            Self::Light => "light",
            Self::Bake => "bake",
            Self::Ui => "ui",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDefinition {
    pub ty: TypeId,
    procedures: [Option<ProcId>; ProfileHook::ALL.len()],
}

impl Default for ProfileDefinition {
    fn default() -> Self {
        Self {
            ty: TypeId::ROOT,
            procedures: [None; ProfileHook::ALL.len()],
        }
    }
}

impl ProfileDefinition {
    pub fn resolve(tree: &ObjectTree, ty: TypeId) -> Self {
        let procedures = ProfileHook::ALL.map(|hook| {
            tree.proc_inherited(ty, &Identifier::from(hook.name()))
                .and_then(|procedure| procedure.body)
        });

        Self { ty, procedures }
    }

    pub fn procedure(&self, hook: ProfileHook) -> Option<ProcId> { self.procedures[hook as usize] }

    pub fn declaration<'tree>(&self, tree: &'tree ObjectTree, hook: ProfileHook) -> Option<&'tree ProcDecl> {
        tree.proc_inherited(self.ty, &Identifier::from(hook.name()))
    }

    pub fn entry_points(&self) -> Vec<ProcId> {
        self.procedures
            .into_iter()
            .flatten()
            .fold(Vec::new(), |mut procedures, procedure| {
                if !procedures.contains(&procedure) {
                    procedures.push(procedure);
                }
                procedures
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Missing,
    MissingDefault(Vec<String>),
    MultipleDefaults(Vec<String>),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(
                formatter,
                "the codebase defines no subtype of {BASE_PATH} under #ifdef __DEMIR_BAKE__"
            ),
            Self::MissingDefault(paths) => write!(
                formatter,
                "the codebase defines {} profiles but none directly sets {DEFAULT_VARIABLE} to a true value: {}",
                paths.len(),
                paths.join(", ")
            ),
            Self::MultipleDefaults(paths) => write!(
                formatter,
                "the codebase defines {} default profiles: {}",
                paths.len(),
                paths.join(", ")
            ),
        }
    }
}

impl std::error::Error for ProfileError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCatalog {
    pub profiles: Vec<TypeId>,
    pub default: TypeId,
}

impl ProfileCatalog {
    pub fn select(&self, tree: &ObjectTree, requested: Option<&TreePath>) -> TypeId {
        requested
            .and_then(|path| tree.id_of(path))
            .filter(|id| self.profiles.contains(id))
            .unwrap_or(self.default)
    }
}

/// every single descendants of [`BASE_PATH`] is also considered profile by default
pub fn catalog(tree: &ObjectTree) -> Result<ProfileCatalog, ProfileError> {
    let base = tree.id_of(&TreePath::parse(BASE_PATH)).ok_or(ProfileError::Missing)?;

    let mut profiles = tree
        .descendants(base)
        .into_iter()
        .filter(|id| *id != base)
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        let left = tree.get(*left).map(|decl| decl.path.to_string()).unwrap_or_default();
        let right = tree.get(*right).map(|decl| decl.path.to_string()).unwrap_or_default();

        left.cmp(&right)
    });

    if profiles.is_empty() {
        return Err(ProfileError::Missing);
    }

    let marker = Identifier::from(DEFAULT_VARIABLE);
    let defaults = profiles
        .iter()
        .copied()
        .filter(|id| {
            tree.var(*id, &marker)
                .is_some_and(|variable| variable.initializer.is_none() && variable.value.is_truthy())
        })
        .collect::<Vec<_>>();

    let paths = |types: &[TypeId]| {
        types
            .iter()
            .filter_map(|id| tree.get(*id).map(|decl| decl.path.to_string()))
            .collect::<Vec<_>>()
    };
    let default = match defaults.as_slice() {
        [] => return Err(ProfileError::MissingDefault(paths(&profiles))),
        [default] => *default,
        _ => return Err(ProfileError::MultipleDefaults(paths(&defaults))),
    };

    Ok(ProfileCatalog { profiles, default })
}

pub fn default_type(tree: &ObjectTree) -> Result<TypeId, ProfileError> { Ok(catalog(tree)?.default) }

pub fn selected_type(tree: &ObjectTree, requested: Option<&TreePath>) -> Result<TypeId, ProfileError> {
    let catalog = catalog(tree)?;

    Ok(catalog.select(tree, requested))
}

pub fn exists(tree: &ObjectTree) -> bool { default_type(tree).is_ok() }
