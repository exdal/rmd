use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use dmm::Prefab;
use objtree::TypeId;
use render::{FrameUpdate, texture::TextureCatalog};

use crate::{
    baker::Baker,
    document::{DocumentId, PrefabInstanceId},
    frame::{FrameInstances, FrameOptions, TypeVisibility},
    git::{session::GitDocState, worker::GitWorker},
    maploader::preview::ModulePreviewState,
    state::EditorState,
    visual,
};

mod bake;
mod clipboard;
mod codebase;
mod coop;
mod direction;
mod document;
mod edit;
#[cfg(test)]
pub(crate) mod fixtures;
mod focus;
mod frame;
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
    bake::report_bake_output, direction::direction_state, edit::is_reorder_label, highlight::always_highlighted,
    node::NodeEditState, preview::RegionPreviewCache, region::RegionState,
};
pub(crate) use self::{
    bake::{FaultState, RuntimeFault},
    codebase::{DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS, build_textures, discover_maps},
    coop::{Activity, Coop, CoopStatus, SharedState},
    direction::{DirectionState, DirectionalTypes},
    edit::context_placement_group,
    frame::TypeLayer,
    guides::GuideBadge,
    instance::{EditScope, GizmoMember, PickedTransform},
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
pub(crate) struct DocumentCache {
    pub(crate) modules: ModulePreviewState,
    pub(crate) node_types: HashSet<TypeId>,
    pub(crate) instances: FrameInstances,
    pub(crate) revision: u64,
    pub(crate) frame_updates: Vec<FrameUpdate>,
    pub(crate) lighting_revision: u64,
    pub(crate) lighting_updates: Vec<render::LightingUpdate>,
    preview: Option<RegionPreviewCache>,
    pub(crate) bake: Option<crate::bake::Bake>,
    bake_hold: BakeHold,
    // edited while the bake was off growing by a level, replayed when it comes back
    pub(crate) unbaked: Vec<PrefabInstanceId>,
    pub(crate) always_highlights: Vec<PrefabInstanceId>,
    unknown_highlights: RefCell<highlight::UnknownHighlights>,
    issues: RefCell<issues::IssueGroups>,
    /// What a replayed panel frame asked this document to re-derive, held until it is active.
    pub(crate) pending_rebake: crate::bake::UiRebake,
    pub(crate) git: Option<GitDocState>,
    pub(crate) map_revision: u64,
    runtimes: Vec<bake::LoggedFault>,
}

pub(crate) type SharedHighlights = Arc<[crate::bake::Highlight]>;

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    pub diagnostics: crate::environment::LoadDiagnostics,
    pub(crate) caches: HashMap<DocumentId, DocumentCache>,
    pub(crate) next_revision: u64,
    pub(crate) next_preview_revision: u64,
    pub(crate) type_visibility: TypeVisibility,
    pub(crate) type_thumbnails: HashMap<TypeId, Option<PrefabThumbnail>>,
    pub(crate) texture_revision: u64,
    pub(crate) maps: Vec<PathBuf>,
    pub(crate) baker: Baker,
    pub(crate) git_worker: GitWorker,
    pub(crate) git_enabled: bool,
    pub(crate) sanitize_vars_on_save: bool,
    /// A map whose merge conflicts were just loaded, for the UI to surface
    pub(crate) loaded_conflicts: Option<DocumentId>,
    pub(crate) loaded_issues: Option<DocumentId>,
    pub(crate) queued_bakes: Vec<DocumentId>,
    pub(crate) standalone_baker: crate::bake::Standalone,
    pub(crate) standalone: Vec<(Prefab, Option<visual::Appearance>)>,
    pub(crate) logged_faults: HashSet<vm::FaultKey>,
    pub(crate) logged_ui_fault: Option<vm::FaultKey>,
    /// The last interaction the panel committed, replayed into a bake that lands after it.
    pub(crate) ui_feedback: Option<crate::bake::UiFeedback>,
    node_edit: Option<NodeEditState>,
    identical: Option<instance::IdenticalCache>,
    pub(crate) coop: Option<Coop>,
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
            diagnostics: crate::environment::LoadDiagnostics::default(),
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
            standalone_baker: crate::bake::Standalone::default(),
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
