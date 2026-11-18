use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use base::{
    types::{Identifier, Value},
    vars,
};

use dmm::{Map, error::MapError};
use objtree::{ObjectTree, TypeId};
use prefab::Prefab;
use render::texture::TextureCatalog;

use crate::{
    app::external_editor::SourceLocation,
    environment::{
        Environment, is_map,
        progress::{Progress, Stage},
        worker::LoadedCodebase,
        world::World,
    },
    frame::{options::TypeVisibility, visual},
    session::Session,
    tool::Tool,
};

pub(crate) const MAX_REPORTED_DIAGNOSTICS: usize = 500;

pub(crate) fn discover_maps(root: &Path) -> Vec<PathBuf> {
    let mut maps = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };

            if kind.is_dir() {
                if !is_hidden(&path) {
                    pending.push(path);
                }
            } else if kind.is_file() && is_map(&path) {
                maps.push(path);
            }
        }
    }

    maps.sort();

    maps
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

pub(crate) fn build_textures(environment: &Environment, progress: &Progress) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    textures
        .insert_missing_icon()
        .expect("one built-in texture fits in the catalog");
    let names = environment.icon_paths();

    progress.enter(Stage::Textures, names.len());
    for name in names {
        if progress.is_cancelled() {
            break;
        }
        progress.advance(name);

        let Some(info) = environment.icon_info.get(name) else {
            continue;
        };
        if let Err(e) = textures.insert_info(name, info) {
            log::warn!("{e}");
        }
    }

    textures
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LoadReport {
    pub warnings: usize,
    pub errors: usize,
    pub warning_lines: Vec<String>,
    pub error_lines: Vec<String>,
}

impl LoadReport {
    pub fn is_empty(&self) -> bool {
        self.warnings + self.errors == 0
    }

    pub fn count(&self, severity: DiagnosticSeverity) -> usize {
        match severity {
            DiagnosticSeverity::Warning => self.warnings,
            DiagnosticSeverity::Error => self.errors,
        }
    }

    pub fn lines(&self, severity: DiagnosticSeverity) -> &[String] {
        match severity {
            DiagnosticSeverity::Warning => &self.warning_lines,
            DiagnosticSeverity::Error => &self.error_lines,
        }
    }

    fn record(&mut self, severity: DiagnosticSeverity, line: String) {
        let (count, lines) = match severity {
            DiagnosticSeverity::Warning => (&mut self.warnings, &mut self.warning_lines),
            DiagnosticSeverity::Error => (&mut self.errors, &mut self.error_lines),
        };
        *count += 1;
        if lines.len() < MAX_REPORTED_DIAGNOSTICS {
            lines.push(line);
        }
    }

    pub fn map(path: &str, map: &Map, tree: Option<&ObjectTree>, errors: &[MapError]) -> Self {
        let mut report = Self::default();
        for error in errors {
            report.record(DiagnosticSeverity::Error, format!("{path}: {error}"));
        }

        if let Some(tree) = tree {
            let used = map.grid.iter().flatten().flatten().copied().collect::<HashSet<_>>();
            let placed = map
                .dictionary
                .iter()
                .filter(|(key, _)| used.contains(key))
                .flat_map(|(_, tile)| tile.iter().map(|prefab| &prefab.path))
                .collect::<HashSet<_>>();
            let unknown = map
                .dictionary
                .values()
                .flatten()
                .filter(|prefab| !placed.contains(&prefab.path) && tree.id_of(&prefab.path).is_none())
                .map(|prefab| prefab.path.to_string())
                .collect::<BTreeSet<_>>();

            for type_path in unknown {
                report.record(
                    DiagnosticSeverity::Warning,
                    format!("{path}: unknown type path {type_path}"),
                );
            }
        }

        report
    }

    pub fn failure(display_path: &str, original_path: &Path, error: &str) -> Self {
        let mut report = Self::default();
        let prefix = format!("{}: ", original_path.display());
        let detail = error.strip_prefix(&prefix).unwrap_or(error);
        report.record(DiagnosticSeverity::Error, format!("{display_path}: {detail}"));
        report
    }
}

fn preprocess_severity(level: preprocessor::diagnostic::Level) -> DiagnosticSeverity {
    if level == preprocessor::diagnostic::Level::Error {
        DiagnosticSeverity::Error
    } else {
        DiagnosticSeverity::Warning
    }
}

pub(crate) fn report(environment: &Environment, diagnostics: &crate::environment::LoadDiagnostics) -> LoadReport {
    let root = environment.base_dir();
    let path = |file| {
        let path = environment.file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };
    let bake_path = |file| {
        let path = environment.bake_file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };
    let mut report = LoadReport::default();
    let mut collect = |severity, line: String, prefix: &str| {
        let level = match severity {
            DiagnosticSeverity::Warning => log::Level::Warn,
            DiagnosticSeverity::Error => log::Level::Error,
        };
        log::log!(level, "{}", line.strip_prefix(prefix).unwrap_or(&line));
        report.record(severity, line);
    };

    for error in &diagnostics.preprocess {
        collect(
            preprocess_severity(error.level),
            error.display(path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.sema {
        collect(
            DiagnosticSeverity::Error,
            error.display(path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.bake_preprocess {
        collect(
            preprocess_severity(error.level),
            error.display(bake_path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.bake_sema {
        collect(
            DiagnosticSeverity::Error,
            error.display(bake_path(error.location.file)).to_string(),
            "",
        );
    }

    if let Some(error) = &diagnostics.profile {
        collect(
            DiagnosticSeverity::Warning,
            format!("warning: baking is off, profile selection failed: {error}"),
            "warning: ",
        );
    }

    if let Some(error) = &diagnostics.codegen {
        collect(
            DiagnosticSeverity::Warning,
            format!("warning: baking is off, bytecode generation failed: {error}"),
            "warning: ",
        );
    }

    for (name, error) in &diagnostics.icons {
        collect(
            DiagnosticSeverity::Warning,
            format!("warning: could not read '{name}': {error}"),
            "warning: ",
        );
    }

    report
}

impl Session {
    pub fn apply_codebase(&mut self, loaded: LoadedCodebase) -> LoadReport {
        self.check_coop_codebase(&loaded.environment);
        self.cancel_node_edit();
        if self.state.tool == Tool::Node {
            self.state.tool = Tool::Pick;
        }

        let LoadedCodebase {
            environment,
            diagnostics,
            textures,
            thumbnails,
            maps,
        } = loaded;

        let report = report(&environment, &diagnostics);
        self.diagnostics = diagnostics;
        self.logged_faults.clear();
        self.logged_ui_fault = None;
        self.textures = textures;
        self.type_thumbnails = thumbnails;
        self.standalone.clear();
        self.standalone_baker = crate::bake::Standalone::default();
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.type_visibility = TypeVisibility::default();
        self.maps = maps;
        self.state.environment = Some(Arc::new(environment));
        self.rebake_all();
        self.clear_runtimes();

        self.loaded_issues = self
            .state
            .active()
            .into_iter()
            .chain(self.state.document_ids())
            .find(|id| !self.map_issues(*id).is_empty());

        report
    }

    pub fn tree(&self) -> Option<&ObjectTree> {
        self.state.environment.as_ref().map(|environment| &environment.tree)
    }

    pub fn world(&self) -> Option<&World> {
        self.state.environment.as_ref().map(|environment| &environment.world)
    }

    pub(crate) fn type_source(&self, id: TypeId) -> Option<SourceLocation> {
        let environment = self.state.environment.as_ref()?;
        let location = environment.tree.get(id)?.location.get();
        if environment.is_builtin_file(location.file) || location.begin.line == 0 || location.begin.col == 0 {
            return None;
        }

        Some(SourceLocation {
            path: environment.file(location.file)?.to_path_buf(),
            line: location.begin.line as usize,
            column: location.begin.col as usize,
        })
    }

    pub fn codebase_name(&self) -> Option<&str> {
        self.world()?.name.as_deref()
    }

    pub fn environment_path(&self) -> Option<&Path> {
        self.state
            .environment
            .as_ref()
            .map(|environment| environment.root.as_path())
    }

    pub fn codebase_dir(&self) -> Option<&Path> {
        self.state.environment.as_deref().map(Environment::base_dir)
    }

    pub fn maps(&self) -> &[PathBuf] {
        &self.maps
    }

    pub fn icon_metadata(&self, name: &str) -> Option<&dmi::metadata::Metadata> {
        self.state.environment.as_ref()?.icon(name)
    }

    pub(crate) fn icon_state_variants(&self, prefab: &Prefab) -> Vec<Prefab> {
        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };

        let Some(id) = environment.tree.id_of(&prefab.path) else {
            return Vec::new();
        };

        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let Some(metadata) = appearance.icon.as_deref().and_then(|icon| environment.icon(icon)) else {
            return Vec::new();
        };

        let mut seen = HashSet::new();
        metadata
            .states
            .iter()
            .filter(|state| seen.insert(state.name.as_str()))
            .map(|state| {
                let mut variant = prefab.clone();
                variant.set_var(Identifier::from(vars::ICON_STATE), Value::Text(state.name.clone()));
                variant
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
