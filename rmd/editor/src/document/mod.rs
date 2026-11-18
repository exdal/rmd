pub(crate) mod backup;
pub(crate) mod command;
pub(crate) mod grid;
pub(crate) mod instance;
mod journal;
mod level;
pub(crate) mod level_session;
pub(crate) mod panel;
pub(crate) mod patch;
pub(crate) mod selection;
pub(crate) mod session;
pub(crate) mod state;

use base::{
    coord::Coord,
    types::{Identifier, Value},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    io::Write,
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use dmm::{Map, MapFormat, writer};
use objtree::ObjectTree;
use prefab::{Prefab, PrefabInstance, PrefabInstanceId, PrefabLocation};

use crate::{
    environment::world::World,
    focus::AreaFocus,
    maploader::{preview::PREVIEW_ID_START, resolved_path},
    tool::{
        PlacementKind, RegionSelectionMode, SelectionMask, ToolEdit,
        place::{insertion_index, placement_kind, refill_missing_bases},
    },
};

use command::{Edit, EditGroupId, History, LevelEdit, SelectionChange};
use grid::Grid;
use instance::{InstanceChange, PlacedPrefab, PlacedTile, VarMutation};
use journal::Journal;
use selection::Selection;

pub fn sanitize_vars(map: &mut Map, tree: &ObjectTree) -> usize {
    let mut dropped = 0;
    for prefab in map.dictionary.values_mut().flatten() {
        let Some(id) = tree.id_of(&prefab.path) else {
            continue;
        };
        prefab.vars.retain(|(name, variable)| {
            let redundant = variable.value != Value::Unevaluated
                && tree
                    .var_inherited(id, name)
                    .is_some_and(|declaration| declaration.value == variable.value);
            dropped += usize::from(redundant);
            !redundant
        });
    }

    map.dedupe_dictionary();

    dropped
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);

impl DocumentId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);

        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MapDocument {
    id: DocumentId,
    pub path: Option<PathBuf>,
    resolved_path: Option<PathBuf>,
    pub map: Grid,
    baseline: Map,
    format: MapFormat,
    pub history: History,
    pub z: u32,
    needs_initial_save: bool,
    /// Contents differ from the file on disk without any history, like a merge result
    pending_write: bool,
    picked: Vec<PrefabInstanceId>,
    selection: Option<Selection>,
    selection_mode: RegionSelectionMode,
    focus: Option<AreaFocus>,
    generation: u64,
    journal: Option<Journal>,
    read_only: bool,
    preview_covered: Arc<HashSet<Coord>>,
    preview_exempt: HashSet<PrefabInstanceId>,
}

enum StepKind {
    Undo,
    Redo,
    Retract,
}

impl StepKind {
    fn next_step<'a>(&self, history: &'a History) -> Option<&'a Edit> {
        match self {
            StepKind::Undo | StepKind::Retract => history.next_undo(),
            StepKind::Redo => history.next_redo(),
        }
    }

    fn step<'a>(&self, history: &'a mut History, grid: &mut Grid) -> Option<Cow<'a, Edit>> {
        match self {
            StepKind::Undo => history.undo(grid).map(Cow::Borrowed),
            StepKind::Redo => history.redo(grid).map(Cow::Borrowed),
            StepKind::Retract => history.retract(grid).map(Cow::Owned),
        }
    }
}

impl MapDocument {
    pub fn new(map: Map, z: u32) -> Self {
        Self {
            id: DocumentId::new(),
            path: None,
            resolved_path: None,
            map: Grid::from_map(&map),
            format: map.format,
            baseline: map,
            history: History::new(),
            z,
            needs_initial_save: false,
            pending_write: false,
            picked: Vec::new(),
            selection: None,
            selection_mode: RegionSelectionMode::Full,
            focus: None,
            generation: 0,
            journal: None,
            read_only: false,
            preview_covered: Arc::default(),
            preview_exempt: HashSet::new(),
        }
    }

    pub fn start_journal(&mut self) {
        self.journal = Some(Journal::default());
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn is_journaling(&self) -> bool {
        self.journal.is_some()
    }

    pub fn take_journal(&mut self) -> Option<Journal> {
        let journal = self.journal.as_mut().filter(|journal| !journal.is_empty())?;

        Some(std::mem::take(journal))
    }

    pub fn apply_remote(&mut self, tiles: Vec<(Coord, Vec<Prefab>)>) -> Vec<PrefabInstanceId> {
        let mut edit = Edit::new("remote edit");
        let mut affected = Vec::new();
        for (coord, tile) in tiles {
            let Some(current) = self.placed_tile(coord) else {
                continue;
            };

            // unchanged prefabs keep their ids, a new id on an area would repaint its whole region
            let mut unclaimed = current.clone();
            let mut added = Vec::new();
            let placed = tile
                .into_iter()
                .map(
                    |prefab| match unclaimed.iter().position(|placed| *placed.prefab() == prefab) {
                        Some(index) => unclaimed.remove(index),
                        None => {
                            let placed = self.instantiate(prefab);
                            added.push(placed.id());
                            placed
                        },
                    },
                )
                .collect::<PlacedTile>();
            if placed == current {
                continue;
            }

            let is_reordered = added.is_empty() && unclaimed.is_empty();
            if is_reordered {
                affected.extend(placed.iter().map(PlacedPrefab::id));
            } else {
                affected.extend(added);
                affected.extend(unclaimed.iter().map(PlacedPrefab::id));
            }

            edit.change(self, coord, placed);
        }

        affected.sort_unstable();
        affected.dedup();
        // these edits come from remote, not from us: dont do command recording stuff
        command::apply_unrecorded(&mut self.map, &edit);
        self.generation += 1;
        self.pending_write = true;
        self.clear_stale_picked_instance();

        affected
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn selection_mask(&self) -> Option<SelectionMask> {
        self.selection
            .filter(|bounds| bounds.min.z == self.z)
            .map(|bounds| SelectionMask {
                bounds,
                mode: self.selection_mode,
            })
    }

    pub fn open(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        let path = path.into();
        Self {
            resolved_path: Some(resolved_path(&path)),
            path: Some(path),
            ..Self::new(map, z)
        }
    }

    pub fn open_modified(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        let path = path.into();
        Self {
            resolved_path: Some(resolved_path(&path)),
            path: Some(path),
            pending_write: true,
            ..Self::new(map, z)
        }
    }

    pub fn replace_map(&mut self, map: Map, pending_write: bool) {
        // note: this operation drops the history

        let z = self.z.clamp(1, map.size.z.max(1));
        let replaced = Self {
            id: self.id,
            path: self.path.take(),
            resolved_path: self.resolved_path.take(),
            pending_write,
            generation: self.generation + 1,
            ..Self::new(map, z)
        };

        *self = replaced;
    }

    pub fn create(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        let path = path.into();
        Self {
            resolved_path: Some(resolved_path(&path)),
            path: Some(path),
            needs_initial_save: true,
            ..Self::new(map, z)
        }
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.resolved_path = Some(resolved_path(&path));
        self.path = Some(path);
    }

    pub fn resolved_path(&self) -> Option<&Path> {
        self.resolved_path.as_deref()
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string())
    }

    pub fn is_dirty(&self) -> bool {
        self.needs_initial_save || self.pending_write || self.history.is_dirty()
    }

    pub fn needs_initial_save(&self) -> bool {
        self.needs_initial_save
    }

    pub fn mark_unsaved(&mut self) {
        self.pending_write = true;
    }

    pub fn format(&self) -> MapFormat {
        self.format
    }

    pub fn to_map(&self) -> Map {
        self.map.to_map(&self.baseline, self.format)
    }

    pub fn instance_ids_at(&self, coord: Coord) -> &[PrefabInstanceId] {
        self.map.ids_at(coord)
    }

    pub fn instance_location(&self, id: PrefabInstanceId) -> Option<PrefabLocation> {
        self.map.location(id)
    }

    pub fn prefab_instance(&self, id: PrefabInstanceId) -> Option<PrefabInstance<'_>> {
        self.map.prefab_instance(id)
    }

    pub fn prefab_instances(&self) -> impl Iterator<Item = PrefabInstance<'_>> {
        self.map.prefab_instances()
    }

    pub fn picked_instance(&self) -> Option<PrefabInstanceId> {
        self.picked_instances().last()
    }

    pub fn picked_instances(&self) -> impl Iterator<Item = PrefabInstanceId> + '_ {
        self.picked.iter().copied().filter(|id| self.is_pickable_instance(*id))
    }

    pub fn picked_ids(&self) -> &[PrefabInstanceId] {
        &self.picked
    }

    pub fn pick_instance(&mut self, picked: Option<PrefabInstanceId>) {
        self.picked = picked.filter(|id| self.is_pickable_instance(*id)).into_iter().collect();
    }

    pub fn pick_instances(&mut self, ids: &[PrefabInstanceId]) {
        self.picked = ids
            .iter()
            .copied()
            .filter(|id| self.is_pickable_instance(*id))
            .collect();
    }

    pub fn toggle_picked_instance(&mut self, id: PrefabInstanceId) {
        if let Some(index) = self.picked.iter().position(|picked| *picked == id) {
            self.picked.remove(index);
            return;
        }

        if self.is_pickable_instance(id) {
            self.picked.push(id);
        }
    }

    pub fn retain_picked(&mut self, keep: impl FnMut(&PrefabInstanceId) -> bool) {
        self.picked.retain(keep);
    }

    fn is_pickable_instance(&self, id: PrefabInstanceId) -> bool {
        id.get() >= PREVIEW_ID_START || self.instance_location(id).is_some()
    }

    pub fn placed_tile(&self, coord: Coord) -> Option<PlacedTile> {
        let tile = self.map.tile_at(coord)?;
        let ids = self.instance_ids_at(coord);

        Some(
            ids.iter()
                .copied()
                .zip(tile.iter().cloned())
                .map(|(id, prefab)| PlacedPrefab { id, prefab })
                .collect(),
        )
    }

    pub fn instantiate(&mut self, prefab: Prefab) -> PlacedPrefab {
        PlacedPrefab {
            id: self.map.allocate(),
            prefab,
        }
    }

    /// Set one variable on a placed prefab as an undoable map edit.
    ///
    /// Returns `None` when the placement no longer exists and otherwise reports
    /// whether the stored prefab changed.
    pub fn set_instance_var(&mut self, id: PrefabInstanceId, name: Identifier, value: Value) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_instance_vars(id, label, &[VarMutation::Set(name, value)], None)
    }

    /// Apply several variable mutations to one placement as a single map edit.
    /// Reusing a group ID replaces the final state of the previous edit in that
    /// group while retaining its original state, which makes live drags one undo.
    pub fn edit_instance_vars(
        &mut self, id: PrefabInstanceId, label: impl Into<String>, mutations: &[VarMutation],
        group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances(&[id], label, None, mutations, group)
    }

    /// this function keeps the ID stable
    pub fn replace_instance_path(
        &mut self, id: PrefabInstanceId, label: impl Into<String>, path: base::path::TreePath,
        mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances(&[id], label, Some(&path), mutations, group)
    }

    /// Changes every placement in `ids` the same way, as one map edit
    pub fn edit_instances(
        &mut self, ids: &[PrefabInstanceId], label: impl Into<String>, path: Option<&base::path::TreePath>,
        mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let mut by_tile = BTreeMap::<Coord, Vec<usize>>::new();
        for id in ids {
            let location = self.instance_location(*id)?;
            by_tile.entry(location.coord).or_default().push(location.prefab_index);
        }

        let mut edit = Edit::new(label);
        for (coord, indices) in by_tile {
            let mut after = self.placed_tile(coord)?;
            let mut changed = false;
            for index in indices {
                let instance = after.get_mut(index)?;
                let before = instance.prefab().clone();
                if let Some(path) = path {
                    instance.prefab_mut().path = path.clone();
                }
                for mutation in mutations {
                    match mutation {
                        VarMutation::Set(name, value) => instance.prefab_mut().set_var(name.clone(), value.clone()),
                        VarMutation::Remove(name) => {
                            instance.prefab_mut().remove_var(name);
                        },
                    }
                }
                changed |= instance.prefab() != &before;
            }
            if changed {
                edit.change(self, coord, after);
            }
        }
        if edit.is_empty() {
            return Some(false);
        }

        Some(self.apply_grouped(edit, group))
    }

    /// Every placement of exactly `prefab`, on every level
    pub fn identical_instances(&self, prefab: &Prefab) -> Vec<PrefabInstanceId> {
        self.map.identical(prefab)
    }

    /// A moved turf or area replaces the destination's, and a tile left without one gets the world default
    pub fn change_instances(
        &mut self, tree: &ObjectTree, world: &World, changes: &[InstanceChange], label: impl Into<String>,
    ) -> Option<ToolEdit> {
        let size = self.map.size();
        let mut before = BTreeMap::<Coord, PlacedTile>::new();
        let mut batch = Vec::with_capacity(changes.len());

        for change in changes {
            let source_coord = self.instance_location(change.id)?.coord;
            let destination_coord = change.to_coord.filter(|to| {
                to.z == source_coord.z
                    && (1..=size.x.max(1)).contains(&to.x)
                    && (1..=size.y.max(1)).contains(&to.y)
                    && *to != source_coord
            });

            let source_tile = self.placed_tile(source_coord)?;
            before.entry(source_coord).or_insert(source_tile);

            if let Some(destination) = destination_coord {
                before
                    .entry(destination)
                    .or_insert_with(|| self.placed_tile(destination).unwrap_or_default());
            }

            batch.push((change, source_coord, destination_coord));
        }

        let mut after = before.clone();
        let mut affected = changes.iter().map(|change| change.id).collect::<Vec<_>>();
        let mut moved = Vec::new();

        for (change, source_coord, destination_coord) in batch {
            let source_tile = after.get_mut(&source_coord)?;
            let index = source_tile.iter().position(|placed| placed.id == change.id)?;
            let instance = &mut source_tile[index];

            if let Some(path) = &change.path {
                instance.prefab_mut().path = path.clone();
            }

            for mutation in &change.mutations {
                match mutation {
                    VarMutation::Set(name, value) => instance.prefab_mut().set_var(name.clone(), value.clone()),
                    VarMutation::Remove(name) => {
                        instance.prefab_mut().remove_var(name);
                    },
                }
            }

            let Some(destination_coord) = destination_coord else {
                continue;
            };

            moved.push((destination_coord, after.get_mut(&source_coord)?.remove(index)));
        }

        // every mover leaves before any arrives, so a turf only replaces one that stays put
        for (destination_coord, placed) in moved {
            let destination_tile = after.get_mut(&destination_coord)?;
            let kind = placement_kind(tree, placed.prefab()).unwrap_or(PlacementKind::Atom);
            if kind != PlacementKind::Atom {
                destination_tile.retain(|existing| {
                    let is_replaced = placement_kind(tree, existing.prefab()) == Some(kind);
                    if is_replaced {
                        affected.push(existing.id);
                    }

                    !is_replaced
                });
            }

            destination_tile.insert(insertion_index(tree, destination_tile, kind), placed);
        }

        if let Some(base_tile) = &world.base_tile {
            for tile in after.values_mut() {
                affected.extend(refill_missing_bases(self, tree, base_tile, tile));
            }
        }

        let mut edit = Edit::new(label);
        for (coord, after_tile) in after {
            if before.get(&coord) != Some(&after_tile) {
                edit.change(self, coord, after_tile);
            }
        }

        Some(ToolEdit {
            edit,
            picked: None,
            affected,
        })
    }

    pub fn move_instance(
        &mut self, tree: &ObjectTree, id: PrefabInstanceId, to_coord: Coord, label: impl Into<String>,
        mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let location = self.instance_location(id)?;
        let size = self.map.size();
        let in_bounds = to_coord.z == location.coord.z
            && (1..=size.x.max(1)).contains(&to_coord.x)
            && (1..=size.y.max(1)).contains(&to_coord.y);
        if to_coord == location.coord || !in_bounds {
            return self.edit_instance_vars(id, label, mutations, group);
        }

        let mut source = self.placed_tile(location.coord)?;
        if location.prefab_index >= source.len() {
            return None;
        }
        let mut moved = source.remove(location.prefab_index);
        for mutation in mutations {
            match mutation {
                VarMutation::Set(name, value) => moved.prefab_mut().set_var(name.clone(), value.clone()),
                VarMutation::Remove(name) => {
                    moved.prefab_mut().remove_var(name);
                },
            }
        }
        let mut destination = self.placed_tile(to_coord).unwrap_or_default();
        let kind = placement_kind(tree, moved.prefab()).unwrap_or(PlacementKind::Atom);
        destination.insert(insertion_index(tree, &destination, kind), moved);

        let mut edit = Edit::new(label);
        edit.change(self, location.coord, source);
        edit.change(self, to_coord, destination);

        Some(self.apply_grouped(edit, group))
    }

    pub fn set_selection(&mut self, selection: Option<Selection>, selection_mode: RegionSelectionMode) {
        self.selection = selection;
        self.selection_mode = selection_mode;
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    fn restore_selection(&mut self, mask: Option<SelectionMask>) {
        self.selection = mask.map(|mask| mask.bounds);

        if let Some(mask) = mask {
            self.selection_mode = mask.mode;
        }
    }

    pub fn set_focus(&mut self, focus: Option<AreaFocus>) {
        self.focus = focus;
    }

    pub fn focus(&self) -> Option<&AreaFocus> {
        self.focus.as_ref().filter(|focus| focus.seed().z == self.z)
    }

    pub fn focus_on_any_level(&self) -> Option<&AreaFocus> {
        self.focus.as_ref()
    }

    pub fn allows_edit_at(&self, coord: Coord) -> bool {
        !self.read_only && self.is_within_focus(coord) && !self.preview_covered.contains(&coord)
    }

    pub fn allows_select_at(&self, coord: Coord) -> bool {
        !self.read_only && self.is_within_focus(coord)
    }

    pub fn allows_edit_instance(&self, id: PrefabInstanceId) -> bool {
        let Some(location) = self.instance_location(id) else {
            return false;
        };

        !self.read_only
            && self.is_within_focus(location.coord)
            && (self.preview_exempt.contains(&id) || !self.preview_covered.contains(&location.coord))
    }

    pub fn set_preview_covered(&mut self, covered: Arc<HashSet<Coord>>, exempt: HashSet<PrefabInstanceId>) {
        self.preview_covered = covered;
        self.preview_exempt = exempt;
    }

    pub fn is_within_focus(&self, coord: Coord) -> bool {
        self.focus.as_ref().is_none_or(|focus| focus.allows(coord))
    }

    pub fn apply(&mut self, edit: Edit) -> bool {
        self.apply_grouped(edit, None)
    }

    pub fn apply_grouped(&mut self, edit: Edit, group: Option<EditGroupId>) -> bool {
        if self.read_only
            || (!edit.is_structural() && edit.changes.iter().any(|change| !self.is_within_focus(change.coord)))
        {
            return false;
        }

        // kept until the edit lands, a rejected level change would share the whole map again
        let recorded = self.journal.as_ref().map(|_| {
            let mut recorded = Journal::default();
            recorded.record(&edit);

            recorded
        });
        let level = edit.level().map(LevelEdit::z);
        if !self.history.apply_grouped(&mut self.map, edit, group) {
            return false;
        }

        if let (Some(journal), Some(recorded)) = (self.journal.as_mut(), recorded) {
            journal.extend(recorded);
        }

        if let Some(z) = level {
            self.level_changed(z);
        }

        self.generation += 1;
        self.clear_stale_picked_instance();

        true
    }

    fn step_history(&mut self, step_kind: StepKind) -> Option<Vec<PrefabInstanceId>> {
        if self.read_only {
            return None;
        }

        let replaced = step_kind
            .next_step(&self.history)
            .map(|edit| self.current_ids(edit))
            .unwrap_or_default();

        let (level, selection, affected) = {
            let edit = step_kind.step(&mut self.history, &mut self.map)?;
            if let Some(journal) = self.journal.as_mut() {
                journal.record(&edit);
            }

            (
                edit.level().map(LevelEdit::z),
                edit.selection(),
                Some(with_replaced(&edit, replaced)),
            )
        };

        if let Some(z) = level {
            self.level_changed(z);
        }

        if let Some(SelectionChange { before, after }) = selection {
            match step_kind {
                StepKind::Undo | StepKind::Retract => self.restore_selection(before),
                StepKind::Redo => self.restore_selection(after),
            };
        }

        self.generation += 1;
        self.clear_stale_picked_instance();

        affected
    }

    pub fn undo(&mut self) -> bool {
        self.undo_with_affected().is_some()
    }

    pub fn undo_with_affected(&mut self) -> Option<Vec<PrefabInstanceId>> {
        self.step_history(StepKind::Undo)
    }

    pub fn retract_with_affected(&mut self, group: EditGroupId) -> Option<Vec<PrefabInstanceId>> {
        if self.history.undo_group() != Some(group) {
            return None;
        }

        self.step_history(StepKind::Retract)
    }

    pub fn redo(&mut self) -> bool {
        self.redo_with_affected().is_some()
    }

    pub fn redo_with_affected(&mut self) -> Option<Vec<PrefabInstanceId>> {
        self.step_history(StepKind::Redo)
    }

    // edits made elsewhere can leave atoms on these tiles that the history entry never saw
    fn current_ids(&self, edit: &Edit) -> Vec<PrefabInstanceId> {
        edit.changes
            .iter()
            .flat_map(|change| self.map.ids_at(change.coord).iter().copied())
            .collect()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        self.save_with(None)
    }

    pub fn save_with(&mut self, sanitize: Option<&ObjectTree>) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::other("document has no path"));
        };
        let format = self.format;

        self.save_as_with(path, format, sanitize)
    }

    pub fn save_as(&mut self, path: impl Into<PathBuf>, format: MapFormat) -> std::io::Result<()> {
        self.save_as_with(path, format, None)
    }

    pub fn save_as_with(
        &mut self, path: impl Into<PathBuf>, format: MapFormat, sanitize: Option<&ObjectTree>,
    ) -> std::io::Result<()> {
        let path = path.into();
        let exported = self.map.to_map(&self.baseline, format);
        let sanitized = sanitize.map(|tree| {
            let mut sanitized = exported.clone();
            sanitize_vars(&mut sanitized, tree);

            sanitized
        });
        let contents = writer::MapWriter::new(sanitized.as_ref().unwrap_or(&exported))
            .with_format(format)
            .write();

        if self.needs_initial_save {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
            file.write_all(contents.as_bytes())?;
        } else {
            std::fs::write(&path, contents)?;
        }

        // unsanitized, so the next save compares against what the tiles hold
        self.baseline = exported;
        self.format = format;
        self.set_path(path);
        self.needs_initial_save = false;
        self.pending_write = false;
        self.history.mark_saved();

        Ok(())
    }

    pub fn clamp(&self, x: u32, y: u32) -> Coord {
        Coord::new(
            x.clamp(1, self.map.size().x.max(1)),
            y.clamp(1, self.map.size().y.max(1)),
            self.z,
        )
    }

    fn clear_stale_picked_instance(&mut self) {
        let mut picked = mem::take(&mut self.picked);
        picked.retain(|id| self.is_pickable_instance(*id));
        self.picked = picked;
    }
}

fn with_replaced(edit: &Edit, replaced: Vec<PrefabInstanceId>) -> Vec<PrefabInstanceId> {
    let mut affected = edit.affected_instances();
    affected.extend(replaced);
    affected.sort_unstable();
    affected.dedup();

    affected
}

#[cfg(test)]
mod tests;
