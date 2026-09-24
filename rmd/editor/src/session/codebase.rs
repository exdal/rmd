use core::{path::TreePath, types::Identifier};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dmi::IconFile;
use editor::{
    Environment,
    frame::TypeVisibility,
    progress::{Progress, Stage},
    tool::Tool,
};
use objtree::{ObjectTree, TypeId};
use render::texture::TextureCatalog;

use super::Session;
use crate::{external_editor::SourceLocation, loader::LoadedCodebase};

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

fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
}

pub(crate) fn build_textures(environment: &Environment, progress: &Progress) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    textures
        .insert_missing_icon()
        .expect("one built-in texture fits in the catalog");
    let base = environment.base_dir();
    let names = environment.icon_paths();

    progress.enter(Stage::Textures, names.len());
    for name in names {
        if progress.is_cancelled() {
            break;
        }
        progress.advance(name);

        if !environment.icons.contains_key(name) {
            continue;
        }

        let mut candidates =
            std::iter::once(base.join(name)).chain(environment.resource_dirs.iter().map(|dir| dir.join(name)));

        let Some(path) = candidates.find(|path| path.is_file()) else {
            log::warn!("could not find '{name}' on disk");
            continue;
        };

        match IconFile::load_info(&path) {
            Ok(info) => {
                if let Err(e) = textures.insert_info(name, &info) {
                    log::warn!("{e}");
                }
            },
            Err(e) => log::warn!("could not read '{name}': {e}"),
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
    pub fn is_empty(&self) -> bool { self.warnings + self.errors == 0 }

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

    pub fn map(path: &str, errors: &[dmm::error::MapError]) -> Self {
        let mut report = Self::default();
        for error in errors {
            report.record(DiagnosticSeverity::Error, format!("{path}: {error}"));
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

pub(crate) fn report(environment: &Environment, diagnostics: &editor::environment::LoadDiagnostics) -> LoadReport {
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
        self.cancel_node_edit();
        if self.state.tool == Tool::Node {
            self.state.tool = Tool::Select;
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
        self.textures = textures;
        self.type_thumbnails = thumbnails;
        self.standalone.clear();
        self.standalone_baker = editor::bake::Standalone::default();
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.type_visibility = TypeVisibility::default();
        self.maps = maps;
        self.state.environment = Some(Arc::new(environment));
        self.rebake_all();

        report
    }

    pub fn tree(&self) -> Option<&ObjectTree> { self.state.environment.as_ref().map(|environment| &environment.tree) }

    pub(crate) fn type_source(&self, id: TypeId) -> Option<SourceLocation> {
        let environment = self.state.environment.as_ref()?;
        let location = environment.tree.get(id)?.location;
        if location.begin.line == 0 || location.begin.col == 0 {
            return None;
        }

        Some(SourceLocation {
            path: environment.file(location.file)?.to_path_buf(),
            line: location.begin.line,
            column: location.begin.col,
        })
    }

    pub fn codebase_name(&self) -> Option<&str> {
        let tree = self.tree()?;
        let world = tree.id_of(&TreePath::parse("/world"))?;

        tree.var_inherited(world, &Identifier::from("name"))?
            .value
            .as_text()
            .filter(|name| !name.is_empty())
    }

    pub fn environment_path(&self) -> Option<&Path> {
        self.state
            .environment
            .as_ref()
            .map(|environment| environment.root.as_path())
    }

    pub fn codebase_dir(&self) -> Option<&Path> { self.state.environment.as_deref().map(Environment::base_dir) }

    pub fn maps(&self) -> &[PathBuf] { &self.maps }

    pub fn icon_metadata(&self, name: &str) -> Option<&dmi::metadata::Metadata> {
        self.state.environment.as_ref()?.icon(name)
    }
}

#[cfg(test)]
mod tests {
    use core::{
        location::{FileId, Location, Position},
        path::TreePath,
        types::Value,
    };
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
    };

    use dmi::IconFile;
    use dmm::Coord;
    use editor::{Environment, progress::Progress};
    use objtree::ObjectTree;

    use super::{
        DiagnosticSeverity,
        LoadReport,
        MAX_REPORTED_DIAGNOSTICS,
        build_textures,
        discover_maps,
        preprocess_severity,
    };
    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples},
    };

    #[test]
    fn type_sources_resolve_compiler_locations_to_loaded_files() {
        let location = Location::in_file(FileId(0), Position::new(42, 7), Position::new(42, 16));
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/obj/item/tool"), location);
        let mut environment = Environment::new("station.dme", tree);
        environment.files.push(PathBuf::from("code/items.dm"));
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));

        let source = session.type_source(id).expect("known type source");

        assert_eq!(source.path, PathBuf::from("code/items.dm"));
        assert_eq!(source.line, 42);
        assert_eq!(source.column, 7);
    }

    #[test]
    fn load_report_classifies_preprocessor_levels_and_caps_only_displayed_lines() {
        use preprocessor::diagnostic::Level;

        assert_eq!(preprocess_severity(Level::Notice), DiagnosticSeverity::Warning);
        assert_eq!(preprocess_severity(Level::Warning), DiagnosticSeverity::Warning);
        assert_eq!(preprocess_severity(Level::Error), DiagnosticSeverity::Error);

        let mut report = LoadReport::default();
        assert!(report.is_empty());
        for index in 0..=MAX_REPORTED_DIAGNOSTICS {
            report.record(DiagnosticSeverity::Warning, format!("warning {index}"));
        }
        report.record(DiagnosticSeverity::Error, String::from("error"));
        assert_eq!(report.warnings, MAX_REPORTED_DIAGNOSTICS + 1);
        assert_eq!(report.warning_lines.len(), MAX_REPORTED_DIAGNOSTICS);
        assert_eq!(report.errors, 1);
        assert!(!report.is_empty());
        assert_eq!(report.error_lines, ["error"]);
    }

    #[test]
    fn map_and_failed_load_reports_keep_one_readable_path_prefix() {
        let error = dmm::error::MapError::new(
            dmm::error::MapErrorKind::RaggedGrid,
            core::location::Position::new(2, 3),
        );
        let report = LoadReport::map("maps/level.dmm", &[error]);
        assert_eq!(report.errors, 1);
        assert!(report.error_lines[0].starts_with("maps/level.dmm: Map error"));

        let failed = LoadReport::failure(
            "maps/level.dmm",
            Path::new("C:/game/maps/level.dmm"),
            "C:/game/maps/level.dmm: cannot read",
        );
        assert_eq!(failed.error_lines, ["maps/level.dmm: cannot read"]);
    }

    #[test]
    fn map_discovery_finds_maps_the_environment_never_includes() {
        let maps = discover_maps(&examples());

        assert!(
            maps.iter().any(|map| map.ends_with("test.dmm")),
            "expected test.dmm in {maps:?}"
        );
        assert!(maps.iter().all(|map| map.extension().is_some_and(|e| e == "dmm")));
    }

    #[test]
    fn map_discovery_is_sorted_and_skips_missing_roots() {
        let maps = discover_maps(&examples());
        let mut sorted = maps.clone();
        sorted.sort();

        assert_eq!(maps, sorted);
        assert!(discover_maps(&examples().join("does-not-exist")).is_empty());
    }

    #[test]
    fn environment_loading_packs_every_cell_in_each_dmi() {
        let root = examples();
        let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load environment");
        assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
        let roots = environment.tree.roots();
        let obj = roots.obj.expect("embedded definitions register /obj");
        let atom = roots.atom.expect("embedded definitions register /atom");
        let movable = roots.movable.expect("embedded definitions register /atom/movable");
        assert!(environment.tree.is_subtype_of(obj, atom));
        assert!(environment.tree.is_subtype_of(obj, movable));
        assert_eq!(
            environment
                .tree
                .var_inherited(obj, &"pixel_x".into())
                .map(|variable| &variable.value),
            Some(&Value::Num(0.0)),
        );
        assert_eq!(
            environment
                .tree
                .var_inherited(obj, &"step_x".into())
                .map(|variable| &variable.value),
            Some(&Value::Num(0.0)),
        );
        let file = IconFile::load(root.join("icons/test.dmi")).expect("load icon");

        let textures = build_textures(&environment, &Progress::new());

        assert_eq!(textures.len(), 2);
        assert_eq!(textures.cell_count(), file.cell_count() + 1);
        assert!(textures.missing_icon().is_some());
        assert!((0..file.cell_count()).all(|cell| textures.lookup("icons/test.dmi", cell).is_some()));
    }

    #[test]
    fn reloading_an_environment_resets_type_visibility() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];

        assert!(session.toggle_type_visibility(table));
        assert!(session.instances().unwrap().sprite(selected).is_none());
        session.select_instance(Some(selected));
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Unsaved table"))),
            Some(true)
        );
        assert!(session.state.active_document().unwrap().is_dirty());
        session.load_environment(&root.join("test.dme")).unwrap();
        let reloaded_table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();

        assert!(session.is_type_visible(reloaded_table));
        assert!(session.instances().unwrap().sprite(selected).is_some());
        assert!(session.state.active_document().unwrap().is_dirty());
        assert_eq!(session.selected_instance(), Some(selected));
        assert_render_cache_matches_rebuild(&session);
    }
}
