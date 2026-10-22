use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use dmm::Prefab;
use editor::{
    EditorState,
    document::{DocumentId, PrefabInstanceId},
    frame::{FrameInstances, FrameOptions, TypeVisibility},
    maploader::preview::ModulePreviewState,
    visual,
};
use objtree::TypeId;
use render::{FrameUpdate, texture::TextureCatalog};

use crate::{baker::Baker, git_worker::GitWorker};

mod bake;
mod blame;
mod clipboard;
mod codebase;
mod coop;
mod diff;
mod direction;
mod document;
mod edit;
#[cfg(test)]
pub(crate) mod fixtures;
mod focus;
mod frame;
mod git;
mod guides;
mod highlight;
mod instance;
mod issues;
mod level;
mod modules;
mod node;
mod palette;
mod panel;
mod preview;
mod region;
mod search;

use self::{
    bake::report_bake_output, diff::DiffState, direction::direction_state, edit::is_reorder_label,
    highlight::always_highlighted, node::NodeEditState, preview::RegionPreviewCache, region::RegionState,
};
pub(crate) use self::{
    bake::{FaultState, RuntimeFault},
    blame::{BlameState, blame_color},
    codebase::{DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS, build_textures, discover_maps},
    coop::{Activity, Coop, CoopStatus, SharedState},
    diff::DiffSide,
    direction::{DirectionState, DirectionalTypes},
    edit::context_placement_group,
    frame::TypeLayer,
    git::GitDocState,
    guides::GuideBadge,
    instance::{EditScope, PickedTransform},
    issues::MapIssueGroup,
    level::validate_level,
    node::NodeOverlay,
    palette::{PrefabThumbnail, prefab_thumbnail_for, prefab_thumbnail_or_missing},
    preview::{PlacementPreview, RegionPreviewSource},
};

#[derive(Default)]
enum BakeHold {
    /// Every edit bakes RIGHT NOW
    #[default]
    Off,
    /// Edits bake, module settling waits (this happens when we move maproots/modules through gizmo)
    Modules { pending: HashSet<PrefabInstanceId> },
    /// Nothing bakes until it's released (this happens when we move selection region)
    Everything { pending: HashSet<PrefabInstanceId> },
}

#[derive(Default)]
struct DocumentCache {
    modules: ModulePreviewState,
    node_types: HashSet<TypeId>,
    instances: FrameInstances,
    revision: u64,
    frame_updates: Vec<FrameUpdate>,
    lighting_revision: u64,
    lighting_updates: Vec<render::LightingUpdate>,
    preview: Option<RegionPreviewCache>,
    bake: Option<editor::bake::Bake>,
    bake_hold: BakeHold,
    // edited while the bake was off growing by a level, replayed when it comes back
    unbaked: Vec<PrefabInstanceId>,
    always_highlights: Vec<PrefabInstanceId>,
    unknown_highlights: RefCell<highlight::UnknownHighlights>,
    issues: RefCell<issues::IssueGroups>,
    /// What a replayed panel frame asked this document to re-derive, held until it is active.
    pending_rebake: editor::bake::UiRebake,
    git: Option<GitDocState>,
    map_revision: u64,
    runtimes: Vec<bake::LoggedFault>,
}

type SharedHighlights = Arc<[editor::bake::Highlight]>;

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    pub diagnostics: editor::environment::LoadDiagnostics,
    caches: HashMap<DocumentId, DocumentCache>,
    next_revision: u64,
    next_preview_revision: u64,
    type_visibility: TypeVisibility,
    type_thumbnails: HashMap<TypeId, Option<PrefabThumbnail>>,
    texture_revision: u64,
    maps: Vec<PathBuf>,
    baker: Baker,
    git_worker: GitWorker,
    git_enabled: bool,
    sanitize_vars_on_save: bool,
    /// A map whose merge conflicts were just loaded, for the UI to surface
    loaded_conflicts: Option<DocumentId>,
    loaded_issues: Option<DocumentId>,
    queued_bakes: Vec<DocumentId>,
    standalone_baker: editor::bake::Standalone,
    standalone: Vec<(Prefab, Option<visual::Appearance>)>,
    logged_faults: HashSet<vm::FaultKey>,
    logged_ui_fault: Option<vm::FaultKey>,
    /// The last interaction the panel committed, replayed into a bake that lands after it.
    ui_feedback: Option<editor::bake::UiFeedback>,
    node_edit: Option<NodeEditState>,
    identical: Option<instance::IdenticalCache>,
    coop: Option<Coop>,
    region_state: RegionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillOutcome {
    Applied,
    NoChange,
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LevelChange {
    Changed,
    NewLevelRequested,
    Unchanged,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(),
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            diagnostics: editor::environment::LoadDiagnostics::default(),
            caches: HashMap::new(),
            next_revision: 1,
            next_preview_revision: 1,
            type_visibility: TypeVisibility::default(),
            type_thumbnails: HashMap::new(),
            texture_revision: 0,
            maps: Vec::new(),
            baker: Baker::default(),
            git_worker: GitWorker::default(),
            git_enabled: true,
            sanitize_vars_on_save: false,
            loaded_conflicts: None,
            loaded_issues: None,
            queued_bakes: Vec::new(),
            standalone_baker: editor::bake::Standalone::default(),
            standalone: Vec::new(),
            logged_faults: HashSet::new(),
            logged_ui_fault: None,
            ui_feedback: None,
            node_edit: None,
            identical: None,
            coop: None,
            region_state: RegionState::default(),
        }
    }

    pub fn has_background_work(&self) -> bool {
        self.baker.is_busy() || self.git_worker.is_busy() || self.coop.is_some()
    }
}
