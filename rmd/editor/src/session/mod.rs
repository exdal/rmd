use std::{collections::HashMap, path::PathBuf, sync::Arc};

use dmm::Prefab;
use editor::{
    EditorState,
    document::{DocumentId, PrefabInstanceId},
    frame::{FrameInstances, FrameOptions, TypeVisibility},
    visual,
};
use objtree::TypeId;
use render::{FrameUpdate, texture::TextureCatalog};

use crate::{baker::Baker, git_worker::GitWorker};

mod bake;
mod blame;
mod block;
mod clipboard;
mod codebase;
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
mod level;
mod node;
mod palette;
mod panel;
mod preview;
mod search;

use self::{
    bake::report_bake_output,
    diff::DiffState,
    direction::direction_state,
    edit::is_reorder_label,
    highlight::always_highlighted,
    level::MAX_MAP_DIMENSION,
    node::NodeEditState,
    preview::BlockPreviewCache,
};
pub(crate) use self::{
    blame::{BlameState, blame_color},
    codebase::{DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS, build_textures, discover_maps},
    diff::DiffSide,
    direction::{DirectionState, DirectionalTypes},
    edit::context_placement_group,
    git::GitDocState,
    guides::GuideBadge,
    instance::{EditScope, SelectedTransform},
    level::validate_level,
    node::NodeOverlay,
    palette::{PrefabThumbnail, prefab_thumbnail_for, prefab_thumbnail_or_missing},
    preview::{BlockPreviewSource, PlacementPreview},
};

#[derive(Default)]
struct DocumentCache {
    instances: FrameInstances,
    revision: u64,
    frame_update: Option<FrameUpdate>,
    lighting_revision: u64,
    lighting_update: Option<render::LightingUpdate>,
    preview: Option<BlockPreviewCache>,
    bake: Option<editor::bake::Bake>,
    always_highlights: Vec<PrefabInstanceId>,
    /// What a replayed panel frame asked this document to re-derive, held until it is active.
    pending_rebake: editor::bake::UiRebake,
    git: Option<GitDocState>,
    map_revision: u64,
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
    /// A map whose merge conflicts were just loaded, for the UI to surface
    loaded_conflicts: Option<DocumentId>,
    queued_bakes: Vec<DocumentId>,
    standalone_baker: editor::bake::Standalone,
    standalone: Vec<(Prefab, Option<visual::Appearance>)>,
    ui_fault: Option<vm::FaultKind>,
    /// The last interaction the panel committed, replayed into a bake that lands after it.
    ui_feedback: Option<editor::bake::UiFeedback>,
    node_edit: Option<NodeEditState>,
    identical: Option<instance::IdenticalCache>,
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
            loaded_conflicts: None,
            queued_bakes: Vec::new(),
            standalone_baker: editor::bake::Standalone::default(),
            standalone: Vec::new(),
            ui_fault: None,
            ui_feedback: None,
            node_edit: None,
            identical: None,
        }
    }
}
