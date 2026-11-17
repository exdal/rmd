use base::path::TreePath;

use dmm::{Coord, Prefab};
use net::{DeleteLevel, InsertAt, InsertLevel, LevelContents, LevelOp};
use objtree::{ObjectTree, TypeId};

use crate::session::{LevelChange, Session};
use crate::{
    bake::BakeUpdate,
    document::{DocumentId, patch},
};

pub(crate) fn validate_level(z: u32, levels: u32) -> std::io::Result<()> {
    let levels = levels.max(1);

    if !(1..=levels).contains(&z) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("z level {z} is out of range; map has {levels} level(s)"),
        ));
    }

    Ok(())
}

impl Session {
    pub fn z(&self) -> u32 {
        self.state.active_document().map_or(1, |document| document.z)
    }

    pub fn level_count(&self) -> u32 {
        self.state
            .active_document()
            .map_or(1, |document| document.map.size().z.max(1))
    }

    pub fn set_level(&mut self, z: u32) {
        if let Some(id) = self.state.active() {
            self.set_level_of(id, z);
        }
    }

    pub fn set_level_of(&mut self, id: DocumentId, z: u32) {
        if self
            .state
            .document(id)
            .is_none_or(|document| z == document.z || !(1..=document.map.size().z.max(1)).contains(&z))
        {
            return;
        }

        if self.state.active() == Some(id) {
            self.cancel_node_edit();
        }
        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        document.z = z;
    }

    pub fn can_change_level(&self, delta: i32) -> bool {
        let Some(document) = self.state.active_document() else {
            return false;
        };

        if delta < 0 {
            return document.z > 1;
        }
        if delta == 0 {
            return false;
        }

        let levels = document.map.size().z.max(1);
        document.z < levels || self.tree().is_some()
    }

    pub fn change_level(&mut self, delta: i32) -> LevelChange {
        if delta == 0 {
            return LevelChange::Unchanged;
        }

        let Some(id) = self.state.active() else {
            return LevelChange::Unchanged;
        };
        let Some(document) = self.state.document(id) else {
            return LevelChange::Unchanged;
        };

        let current = document.z;
        let levels = document.map.size().z.max(1);
        let target = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs()).max(1)
        } else {
            current.saturating_add(delta as u32)
        };
        if target > levels {
            return LevelChange::NewLevelRequested;
        }

        let next = target.min(levels);

        if next != current {
            self.cancel_node_edit();
            let Some(document) = self.state.document_mut(id) else {
                return LevelChange::Unchanged;
            };
            document.z = next;

            LevelChange::Changed
        } else {
            LevelChange::Unchanged
        }
    }

    pub fn create_level(&mut self, id: DocumentId, fill: &[Prefab]) -> Result<u32, String> {
        self.cancel_node_edit();

        let document = self
            .state
            .document_mut(id)
            .ok_or_else(|| String::from("the map is no longer open"))?;
        let levels = document.map.size().z.max(1);
        if document.z != levels {
            return Err(String::from("the map is no longer on its highest Z level"));
        }

        if document.is_read_only() {
            return Err(String::from("the map is read-only"));
        }

        if self.is_coop_level_requested(id) {
            return Err(String::from("the last level change has not arrived yet"));
        }

        let new_level = LevelOp::Insert(InsertLevel {
            at: InsertAt::Top,
            contents: LevelContents::Fill(patch::encode_tile(fill)),
        });
        if self.request_coop_level(id, new_level) {
            return Ok(levels + 1);
        }

        let z = self
            .state
            .document_mut(id)
            .and_then(|document| document.append_level(fill))
            .ok_or_else(|| String::from("could not allocate another Z level"))?;
        self.bake_appended_level(id, z);
        self.show_level(id, z);

        Ok(z)
    }

    pub(crate) fn has_loaded_conflicts(&self, id: DocumentId) -> bool {
        self.caches
            .get(&id)
            .and_then(|cache| cache.git.as_ref())
            .is_some_and(|git| git.conflicts.is_some())
    }

    pub fn can_delete_level(&self, id: DocumentId) -> bool {
        self.state
            .document(id)
            .is_some_and(|document| !document.is_read_only() && document.map.size().z > 1)
            && !self.has_loaded_conflicts(id)
            && !self.is_coop_level_requested(id)
    }

    pub fn delete_level(&mut self, id: DocumentId, z: u32) -> Result<(), String> {
        if !self.can_delete_level(id) {
            return Err(String::from(
                "the level cannot be deleted: the map must be writable, have another level, and have no loaded merge \
                 conflicts or pending level change",
            ));
        }

        let levels = self.state.document(id).map_or(0, |document| document.map.size().z);
        if !(1..=levels).contains(&z) {
            return Err(String::from("the level is no longer available"));
        }

        if self.request_coop_level(id, LevelOp::Delete(DeleteLevel { z })) {
            return Ok(());
        }

        self.cancel_node_edit();

        let document = self.state.document_mut(id).ok_or("the map is no longer open")?;
        if !document.delete_level(z) {
            return Err(String::from("the level is no longer available"));
        }

        // do a full rebake, multiz lighting needs this
        self.rebake_levels(id);

        Ok(())
    }

    pub(crate) fn rebake_levels(&mut self, id: DocumentId) {
        let cache = self.caches.entry(id).or_default();
        cache.map_revision = cache.map_revision.wrapping_add(1);
        self.rebake(id);
    }

    pub(crate) fn show_level(&mut self, id: DocumentId, z: u32) {
        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        document.z = z;
        document.set_focus(None);
        self.set_active_document(id);
    }

    pub(crate) fn bake_appended_level(&mut self, id: DocumentId, z: u32) {
        let Some(document) = self.state.document(id) else {
            return;
        };

        let size = document.map.size();
        let ids = (1..=size.y)
            .flat_map(|y| (1..=size.x).map(move |x| Coord::new(x, y, z)))
            .flat_map(|coord| document.instance_ids_at(coord).to_vec())
            .collect::<Vec<_>>();

        let cache = self.caches.entry(id).or_default();
        cache.map_revision = cache.map_revision.wrapping_add(1);
        self.bake_new_levels(id);
        // drawn unbaked until the bake comes back, like a freshly opened map
        self.apply_bake_update(
            id,
            BakeUpdate {
                appearances: ids,
                lighting: None,
            },
        );
    }
}

impl Session {
    pub fn resize_map(&mut self, width: u32, height: u32, fill: &[Prefab]) -> Result<(), String> {
        let id = self.state.active().ok_or_else(|| String::from("no map is open"))?;

        // a resize reshares the map
        if self.is_coop_level_requested(id) {
            return Err(String::from("the last level change has not arrived yet"));
        }

        if self.node_edit.as_ref().is_some_and(|edit| edit.document == id) {
            self.cancel_node_edit();
        }

        let resized = self
            .state
            .document_mut(id)
            .is_some_and(|document| document.resize(width, height, fill));
        if resized {
            self.rebake(id);
            let cache = self.caches.entry(id).or_default();
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }

        Ok(())
    }

    /// Tiles a resize would delete that hold more than `fill`
    pub fn resize_losses(&self, width: u32, height: u32, fill: &[Prefab]) -> usize {
        let Some(document) = self.state.active_document() else {
            return 0;
        };
        let size = document.map.size();
        let mut losses = 0;
        for z in 1..=size.z {
            for y in 1..=size.y {
                for x in (1..=size.x).filter(|x| *x > width || y > height) {
                    if document
                        .map
                        .tile_at(Coord::new(x, y, z))
                        .is_some_and(|tile| tile.iter().ne(fill))
                    {
                        losses += 1;
                    }
                }
            }
        }

        losses
    }

    pub(crate) fn default_fill(&self) -> Result<Vec<Prefab>, String> {
        self.world()
            .and_then(|world| world.base_tile.as_ref())
            .map(|base_tile| vec![Prefab::new(base_tile.turf.clone()), Prefab::new(base_tile.area.clone())])
            .ok_or_else(|| String::from("the codebase does not define usable /turf and /area types"))
    }

    pub(crate) fn tile_fill(&self, turf: &str, area: &str) -> Result<Vec<Prefab>, String> {
        let tree = self.tree().ok_or_else(|| String::from("no codebase is loaded"))?;
        let roots = tree.roots();

        Ok(vec![
            Prefab::new(resolve_fill_path(tree, turf, roots.turf, "turf")?),
            Prefab::new(resolve_fill_path(tree, area, roots.area, "area")?),
        ])
    }
}

fn resolve_fill_path(tree: &ObjectTree, input: &str, root: Option<TypeId>, kind: &str) -> Result<TreePath, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(format!("enter a {kind} type path"));
    }

    tree.id_of(&TreePath::parse(input))
        .filter(|id| root.is_some_and(|root| tree.is_subtype_of(*id, root)))
        .and_then(|id| tree.get(id))
        .map(|declaration| TreePath::parse(&declaration.path.to_string()))
        .ok_or_else(|| format!("not a /{kind} type: {input}"))
}

#[cfg(test)]
mod tests;
