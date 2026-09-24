use editor::{
    document::{DocumentId, PrefabInstanceId, Selection},
    search::{self, SearchQuery},
};
use objtree::ObjectTree;

use super::Session;

impl Session {
    pub fn find_instances(
        &self, document: DocumentId, query: &SearchQuery, bounds: Option<Selection>,
    ) -> Vec<PrefabInstanceId> {
        let Some(document) = self.state.document(document) else {
            return Vec::new();
        };

        match self.tree() {
            Some(tree) => search::find_instances(document, tree, query, bounds),
            None => search::find_instances(document, &ObjectTree::default(), query, bounds),
        }
    }

    pub fn delete_instances(&mut self, document: DocumentId, instances: &[PrefabInstanceId]) -> bool {
        if !self.set_active_document(document) {
            return false;
        }

        self.state
            .active_document()
            .and_then(|document| search::delete_instances(document, instances))
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn replace_instances(&mut self, document: DocumentId, instances: &[PrefabInstanceId]) -> bool {
        if !self.set_active_document(document) {
            return false;
        }
        let Some(prefab) = self.palette().cloned() else {
            return false;
        };

        self.state
            .active_pair()
            .and_then(|(environment, document)| {
                search::replace_instances(document, &environment.tree, instances, &prefab)
            })
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn can_replace_instance(&self, document: DocumentId, instance: PrefabInstanceId) -> bool {
        let (Some(tree), Some(document), Some(prefab)) = (self.tree(), self.state.document(document), self.palette())
        else {
            return false;
        };

        search::can_replace(document, tree, instance, prefab)
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Prefab};
    use editor::{search::SearchQuery, tool::Tool};

    use crate::session::fixtures::{assert_render_cache_matches_rebuild, flat_session};

    #[test]
    fn replace_all_swaps_matching_instances_for_the_brush_in_one_undo() {
        let mut session = flat_session(3, 1);
        let document = session.state.active().unwrap();
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        session.state.choose_prefab(table.clone());
        session.set_tool(Tool::Place);
        for x in 1..=3 {
            assert!(session.place_at(Coord::new(x, 1, 1), None).is_some());
        }
        let grid = session.map().unwrap().grid.clone();
        let query = SearchQuery::Type {
            path: TreePath::parse("/obj"),
            subtypes: true,
        };
        let tables = session.find_instances(document, &query, None);
        assert_eq!(tables.len(), 3);

        let mut named = table.clone();
        named.set_var("name".into(), Value::Text("Poker".into()));
        session.state.choose_prefab(named.clone());
        assert!(session.replace_instances(document, &tables));

        let replaced = session.find_instances(document, &SearchQuery::Prefab(named.clone()), None);
        assert_eq!(replaced, tables, "the placements keep their ids");
        assert_render_cache_matches_rebuild(&session);

        assert!(session.undo());
        assert_eq!(session.map().unwrap().grid, grid);
    }

    #[test]
    fn replace_all_skips_instances_of_another_kind() {
        let mut session = flat_session(2, 1);
        let document = session.state.active().unwrap();
        let floors = session.find_instances(
            document,
            &SearchQuery::Prefab(Prefab::new(TreePath::parse("/turf/open/floor"))),
            None,
        );
        assert_eq!(floors.len(), 2);

        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        assert!(!session.can_replace_instance(document, floors[0]));
        assert!(!session.replace_instances(document, &floors));

        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));
        assert!(session.can_replace_instance(document, floors[0]));
        assert!(session.replace_instances(document, &floors));
        assert_eq!(
            session.map().unwrap().tile_at(Coord::new(2, 1, 1)).unwrap()[0].path,
            TreePath::parse("/turf/closed/wall")
        );
    }

    #[test]
    fn delete_all_removes_every_match_and_updates_the_render_cache() {
        let mut session = flat_session(3, 1);
        let document = session.state.active().unwrap();
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        session.state.choose_prefab(table.clone());
        session.set_tool(Tool::Place);
        for x in 1..=3 {
            assert!(session.place_at(Coord::new(x, 1, 1), None).is_some());
        }
        let tables = session.find_instances(document, &SearchQuery::Prefab(table.clone()), None);

        assert!(session.delete_instances(document, &tables));

        assert!(
            session
                .find_instances(document, &SearchQuery::Prefab(table), None)
                .is_empty()
        );
        assert_eq!(
            session.state.active_document().unwrap().undo_label(),
            Some("delete 3 instances")
        );
        assert_render_cache_matches_rebuild(&session);
    }
}
