use core::path::TreePath;

use dmm::{Coord, Prefab};
use editor::{document::DocumentId, tool::default_tile_paths};
use objtree::{ObjectTree, TypeId};

use super::{LevelChange, Session};

pub(super) const MAX_MAP_DIMENSION: u32 = 255;

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
    pub fn z(&self) -> u32 { self.state.active_document().map_or(1, |document| document.z) }

    pub fn level_count(&self) -> u32 {
        self.state
            .active_document()
            .map_or(1, |document| document.map.size.z.max(1))
    }

    pub fn set_level(&mut self, z: u32) {
        if self.z() != z {
            self.cancel_node_edit();
        }
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        if z == document.z || !(1..=document.map.size.z.max(1)).contains(&z) {
            return;
        }

        document.z = z;
        document.set_focus(None);
        document.selection = None;
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

        let levels = document.map.size.z.max(1);
        document.z < levels || (levels < MAX_MAP_DIMENSION && self.tree().is_some())
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
        let levels = document.map.size.z.max(1);
        let target = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs()).max(1)
        } else {
            current.saturating_add(delta as u32)
        };
        if target > levels && levels < MAX_MAP_DIMENSION {
            return LevelChange::NewLevelRequested;
        }

        let next = target.min(levels);

        if next != current {
            self.cancel_node_edit();
            let Some(document) = self.state.document_mut(id) else {
                return LevelChange::Unchanged;
            };
            document.z = next;
            document.set_focus(None);
            document.selection = None;

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
        let levels = document.map.size.z.max(1);
        if document.z != levels {
            return Err(String::from("the map is no longer on its highest Z level"));
        }
        if levels >= MAX_MAP_DIMENSION {
            return Err(format!("maps cannot exceed {MAX_MAP_DIMENSION} Z levels"));
        }

        let z = document
            .append_level(fill)
            .ok_or_else(|| String::from("could not allocate another Z level"))?;
        document.z = z;
        document.set_focus(None);
        document.selection = None;
        self.set_active_document(id);
        self.rebake(id);

        Ok(z)
    }
}

impl Session {
    pub fn resize_map(&mut self, width: u32, height: u32, fill: &[Prefab]) -> Result<(), String> {
        if !(1..=MAX_MAP_DIMENSION).contains(&width) || !(1..=MAX_MAP_DIMENSION).contains(&height) {
            return Err(format!("map dimensions must each be between 1 and {MAX_MAP_DIMENSION}"));
        }
        let id = self.state.active().ok_or_else(|| String::from("no map is open"))?;
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
        let size = document.map.size;
        let mut losses = 0;
        for z in 1..=size.z {
            for y in 1..=size.y {
                for x in (1..=size.x).filter(|x| *x > width || y > height) {
                    if document
                        .map
                        .tile_at(Coord::new(x, y, z))
                        .is_some_and(|tile| tile.as_slice() != fill)
                    {
                        losses += 1;
                    }
                }
            }
        }

        losses
    }

    pub(crate) fn default_fill(&self) -> Result<Vec<Prefab>, String> {
        self.tree()
            .and_then(default_tile_paths)
            .map(|(turf, area)| vec![Prefab::new(turf), Prefab::new(area)])
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
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{
        document::{MapDocument, Selection},
        tool::Tool,
    };

    use super::{MAX_MAP_DIMENSION, validate_level};
    use crate::session::{
        LevelChange,
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session},
    };

    #[test]
    fn resizing_the_map_rebuilds_the_render_cache_and_undo_restores_it() {
        let mut session = flat_session(3, 2);
        let fill = session.default_fill().unwrap();
        let tile = session.options.tile_size as f32;

        assert_eq!(session.resize_map(5, 4, &fill), Ok(()));
        let size = session.map().unwrap().size;
        assert_eq!((size.x, size.y), (5, 4));
        assert_eq!(session.map().unwrap().tile_at(Coord::new(5, 4, 1)), Some(&fill));
        assert_eq!(session.extent_px(), (5.0 * tile, 4.0 * tile));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.undo());
        let size = session.map().unwrap().size;
        assert_eq!((size.x, size.y), (3, 2));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.redo());
        assert_eq!(session.map().unwrap().size.x, 5);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn resize_losses_count_tiles_with_more_than_the_fill() {
        let mut session = flat_session(3, 1);
        let fill = session.default_fill().unwrap();
        assert_eq!(session.resize_map(4, 1, &fill), Ok(()));

        assert_eq!(session.resize_losses(3, 1, &fill), 0, "only the added fill tile goes");
        assert_eq!(session.resize_losses(2, 1, &fill), 1);
        assert_eq!(session.resize_losses(4, 1, &fill), 0);
        assert!(session.resize_map(0, 1, &fill).is_err());
        assert!(session.resize_map(1, MAX_MAP_DIMENSION + 1, &fill).is_err());
    }

    #[test]
    fn tile_fill_accepts_only_turf_and_area_subtypes() {
        let session = flat_session(1, 1);
        let fill = session.default_fill().unwrap();
        let (turf, area) = (fill[0].path.to_string(), fill[1].path.to_string());

        assert_eq!(session.tile_fill(&format!(" {turf} "), &area), Ok(fill));
        assert!(session.tile_fill(&area, &area).is_err());
        assert!(session.tile_fill(&turf, "/obj").is_err());
        assert!(session.tile_fill("/turf/missing", &area).is_err());
        assert!(session.tile_fill("", &area).is_err());
    }

    #[test]
    fn validates_one_based_map_levels() {
        assert!(validate_level(1, 3).is_ok());
        assert!(validate_level(3, 3).is_ok());
        assert!(validate_level(0, 3).is_err());
        assert!(validate_level(4, 3).is_err());
    }

    #[test]
    fn changing_levels_clears_focus() {
        let mut session = focus_session();
        session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1),))));
        assert!(session.focused_area().is_some());
        assert!(session.selection().is_some());

        session.change_level(1);

        assert_eq!(session.z(), 2);
        assert_eq!(session.focused_area(), None);
        assert_eq!(session.selection(), None);
        assert!(session.can_edit_at(Coord::new(4, 1, 2)));
    }

    #[test]
    fn changing_up_from_the_last_level_requests_and_uses_a_fill_type() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf")),
            Prefab::new(TreePath::parse("/area")),
        ]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }
        let id = session.activate_document(MapDocument::new(map, 1));
        let revision = session.revision();

        assert!(session.can_change_level(1));
        assert!(!session.can_change_level(-1));
        assert_eq!(session.change_level(1), LevelChange::NewLevelRequested);
        assert_eq!(session.level_count(), 1, "the request does not create a level yet");
        let fill = session.tile_fill("/turf", "/area").unwrap();
        assert_eq!(session.create_level(id, &fill), Ok(2));

        let document = session.state.active_document().expect("active document");
        assert_eq!(document.map.size.z, 2);
        assert_eq!(document.z, 2);
        assert!(document.is_dirty());
        for x in 1..=2 {
            assert_eq!(
                document.map.tile_at(Coord::new(x, 1, 2)),
                Some(&fill),
                "the chosen turf and area"
            );
        }
        assert_ne!(
            session.revision(),
            revision,
            "the appended level is rendered immediately"
        );
        assert_eq!(
            session
                .map_view_frame(id, Default::default(), Default::default(), Default::default(), &[], &[])
                .unwrap()
                .level_count,
            2
        );

        session.change_level(-1);
        assert_eq!(session.z(), 1);
        assert_eq!(
            session.level_count(),
            2,
            "moving down does not delete the appended level"
        );
    }

    #[test]
    fn level_creation_stops_at_the_map_dimension_limit() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        let mut map = Map::new(Size {
            x: 1,
            y: 1,
            z: MAX_MAP_DIMENSION,
        });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf")),
            Prefab::new(TreePath::parse("/area")),
        ]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }
        session.activate_document(MapDocument::new(map, MAX_MAP_DIMENSION));

        assert!(!session.can_change_level(1));
        assert_eq!(session.change_level(1), LevelChange::Unchanged);
        assert_eq!(session.z(), MAX_MAP_DIMENSION);
        assert_eq!(session.level_count(), MAX_MAP_DIMENSION);
    }
}
