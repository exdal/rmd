use core::path::TreePath;

use dmm::Prefab;
use editor::{document::DocumentId, tool::is_placeable};

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

    pub fn create_level(&mut self, id: DocumentId, type_path: &str) -> Result<u32, String> {
        self.cancel_node_edit();
        let type_path = type_path.trim();
        if type_path.is_empty() {
            return Err(String::from("enter a type path"));
        }

        let requested = TreePath::parse(type_path);
        let path = {
            let tree = self.tree().ok_or_else(|| String::from("no codebase is loaded"))?;
            let type_id = tree
                .id_of(&requested)
                .ok_or_else(|| format!("unknown type path: {type_path}"))?;
            if !is_placeable(tree, type_id) {
                return Err(format!("type path is not an atom: {type_path}"));
            }

            tree.get(type_id)
                .map(|declaration| TreePath::parse(&declaration.path.to_string()))
                .ok_or_else(|| format!("unknown type path: {type_path}"))?
        };

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
            .append_level(&[Prefab::new(path)])
            .ok_or_else(|| String::from("could not allocate another Z level"))?;
        document.z = z;
        document.set_focus(None);
        document.selection = None;
        self.set_active_document(id);
        self.rebake(id);

        Ok(z)
    }
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
        fixtures::{examples, focus_session},
    };

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
        assert!(session.create_level(id, "/missing/type").is_err());
        assert_eq!(session.level_count(), 1, "an invalid fill path changes nothing");
        assert_eq!(session.create_level(id, "/turf"), Ok(2));

        let document = session.state.active_document().expect("active document");
        assert_eq!(document.map.size.z, 2);
        assert_eq!(document.z, 2);
        assert!(document.is_dirty());
        for x in 1..=2 {
            assert_eq!(
                document.map.tile_at(Coord::new(x, 1, 2)).expect("filled tile")[0].path,
                TreePath::parse("/turf")
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
