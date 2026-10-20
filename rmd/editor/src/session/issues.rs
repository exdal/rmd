use core::path::TreePath;
use std::{collections::BTreeMap, sync::Arc};

use editor::{
    document::{DocumentId, PrefabInstanceId},
    issues,
};

use super::Session;

#[derive(Debug)]
pub(crate) struct MapIssueGroup {
    pub path: TreePath,
    pub instances: Vec<PrefabInstanceId>,
}

#[derive(Default)]
pub(super) struct IssueGroups {
    key: (u64, u64),
    groups: Arc<[MapIssueGroup]>,
}

impl Session {
    pub(crate) fn map_issues(&self, id: DocumentId) -> Arc<[MapIssueGroup]> {
        let (Some(document), Some(cache)) = (self.state.document(id), self.caches.get(&id)) else {
            return Arc::default();
        };

        let key = (document.generation(), cache.revision);
        let mut issues = cache.issues.borrow_mut();
        if issues.key != key {
            let mut grouped = BTreeMap::<String, MapIssueGroup>::new();
            for id in cache.instances.unknown_type_instances() {
                let Some(instance) = document.prefab_instance(id) else {
                    continue;
                };

                let path = &instance.prefab().path;
                grouped
                    .entry(path.to_string())
                    .or_insert_with(|| MapIssueGroup {
                        path: path.clone(),
                        instances: Vec::new(),
                    })
                    .instances
                    .push(id);
            }

            for group in grouped.values_mut() {
                group.instances.sort_unstable_by_key(|id| {
                    let location = document.instance_location(*id).unwrap();
                    (
                        location.coord.z,
                        location.coord.y,
                        location.coord.x,
                        location.prefab_index,
                        id.get(),
                    )
                });
            }

            issues.groups = grouped.into_values().collect::<Vec<_>>().into();
            issues.key = key;
        }

        Arc::clone(&issues.groups)
    }

    pub(super) fn note_loaded_issues(&mut self, id: DocumentId) {
        if !self.map_issues(id).is_empty() {
            self.loaded_issues = Some(id);
        }
    }

    pub(crate) fn take_loaded_issues(&mut self) -> Option<DocumentId> {
        self.loaded_issues
            .take()
            .filter(|id| self.state.document(*id).is_some())
    }

    pub(crate) fn editable_issue_instances(
        &self, id: DocumentId, path: &TreePath, instances: &[PrefabInstanceId],
    ) -> Vec<PrefabInstanceId> {
        match (self.state.document(id), self.tree()) {
            (Some(document), Some(tree)) => issues::editable_instances(document, tree, path, instances),
            _ => Vec::new(),
        }
    }

    pub(crate) fn replace_issue_instances(
        &mut self, id: DocumentId, path: &TreePath, instances: &[PrefabInstanceId], target: &TreePath,
    ) -> bool {
        let action = self
            .state
            .document(id)
            .zip(self.tree())
            .and_then(|(document, tree)| issues::replace_instances(document, tree, path, instances, target));
        action.is_some_and(|action| self.set_active_document(id) && self.commit(action, None))
    }

    pub(crate) fn delete_issue_instances(
        &mut self, id: DocumentId, path: &TreePath, instances: &[PrefabInstanceId],
    ) -> bool {
        let action = self
            .state
            .document(id)
            .zip(self.tree())
            .and_then(|(document, tree)| issues::delete_instances(document, tree, path, instances));
        action.is_some_and(|action| self.set_active_document(id) && self.commit(action, None))
    }
}

#[cfg(test)]
mod tests {
    use core::location::Location;

    use dmm::{Coord, Prefab};
    use editor::{focus::AreaFocus, progress::Progress};

    use super::*;
    use crate::session::fixtures::{assert_render_cache_matches_rebuild, examples, unknown_paths_session};

    #[test]
    fn groups_are_sorted_live_and_shared_until_the_document_changes() {
        let mut session = unknown_paths_session();
        let id = session.state.active().unwrap();
        let groups = session.map_issues(id);
        assert_eq!(
            groups.iter().map(|group| group.path.to_string()).collect::<Vec<_>>(),
            ["/obj/removed_table", "/turf/removed_floor"]
        );
        assert_eq!(groups[0].instances.len(), 4);
        assert_eq!(groups[1].instances.len(), 2);
        let coords = groups[0]
            .instances
            .iter()
            .map(|owner| {
                session
                    .state
                    .document(id)
                    .unwrap()
                    .instance_location(*owner)
                    .unwrap()
                    .coord
            })
            .collect::<Vec<_>>();
        assert_eq!(
            coords,
            [
                Coord::new(1, 1, 1),
                Coord::new(2, 1, 1),
                Coord::new(1, 1, 2),
                Coord::new(2, 1, 2)
            ]
        );
        assert!(Arc::ptr_eq(&groups, &session.map_issues(id)));
        assert_eq!(session.take_loaded_issues(), Some(id));
        assert_eq!(session.take_loaded_issues(), None);
        session.close_map(id);
        assert!(session.map_issues(id).is_empty());
    }

    fn id_for(session: &Session) -> DocumentId {
        session.state.active().unwrap()
    }

    #[test]
    fn bulk_replacement_preserves_each_override_identity_and_order_with_one_undo() {
        let mut session = unknown_paths_session();
        let id = id_for(&session);
        let groups = session.map_issues(id);
        let group = &groups[0];
        let before = group
            .instances
            .iter()
            .map(|owner| {
                let instance = session.state.document(id).unwrap().prefab_instance(*owner).unwrap();
                (*owner, instance.prefab().clone(), instance.location)
            })
            .collect::<Vec<_>>();
        let target = TreePath::parse("/obj/structure/table");
        assert!(session.replace_issue_instances(id, &group.path, &group.instances, &target));
        for (owner, prefab, location) in before {
            let instance = session.state.document(id).unwrap().prefab_instance(owner).unwrap();
            assert_eq!(instance.prefab().path, target);
            assert_eq!(instance.prefab().vars, prefab.vars);
            assert_eq!(instance.location, location);
        }
        assert_eq!(session.map_issues(id).len(), 1);
        assert_eq!(session.caches[&id].instances.unknown_type_instances().count(), 2);
        assert!(session.state.document(id).unwrap().is_dirty());
        assert_render_cache_matches_rebuild(&session);
        assert!(session.undo());
        assert_eq!(session.map_issues(id)[0].instances, group.instances);
        assert!(session.redo());
        assert_eq!(session.map_issues(id).len(), 1);
        assert!(
            !session.replace_issue_instances(id, &group.path, &group.instances, &target),
            "stale rows cannot be repaired twice"
        );
    }

    #[test]
    fn repairs_target_one_map_even_when_another_map_has_the_same_placement_ids() {
        let mut session = unknown_paths_session();
        let first = id_for(&session);
        let groups = session.map_issues(first);
        let group = &groups[0];
        let map = session.state.document(first).unwrap().to_map();
        session.apply_map(crate::loader::LoadedMap {
            path: "other-unknown-paths.dmm".into(),
            map,
            z: 1,
            errors: Vec::new(),
            repo: None,
            conflict: None,
        });
        let second = id_for(&session);
        assert_ne!(first, second);
        assert_eq!(session.map_issues(second)[0].instances, group.instances);
        assert!(session.replace_issue_instances(
            first,
            &group.path,
            &group.instances,
            &TreePath::parse("/obj/structure/table")
        ));
        assert_eq!(session.state.active(), Some(first));
        assert_eq!(session.map_issues(first).len(), 1);
        assert_eq!(session.map_issues(second)[0].instances, group.instances);
        assert!(!session.state.document(second).unwrap().is_dirty());
        assert!(session.undo());
        assert_eq!(session.map_issues(first)[0].instances, group.instances);
    }

    #[test]
    fn repairs_reject_incompatible_types_and_respect_read_only_maps_and_focus() {
        let mut session = unknown_paths_session();
        let id = id_for(&session);
        let groups = session.map_issues(id);
        let group = &groups[0];
        for target in ["/turf/open/floor", "/area/station", "/obj/also_missing"] {
            assert!(!session.replace_issue_instances(id, &group.path, &group.instances, &TreePath::parse(target)));
        }
        assert!(session.undo_label().is_none());
        session.state.document_mut(id).unwrap().set_read_only(true);
        assert!(
            session
                .editable_issue_instances(id, &group.path, &group.instances)
                .is_empty()
        );
        assert!(!session.delete_issue_instances(id, &group.path, &group.instances));
        assert!(!session.replace_issue_instances(
            id,
            &group.path,
            &group.instances,
            &TreePath::parse("/obj/structure/table")
        ));
        session.state.document_mut(id).unwrap().set_read_only(false);
        let seed = Coord::new(1, 1, 1);
        session.state.document_mut(id).unwrap().set_focus(Some(AreaFocus::new(
            seed,
            Prefab::new(TreePath::parse("/area/station")),
            group.instances[0],
            [seed].into(),
        )));
        assert_eq!(
            session
                .editable_issue_instances(id, &group.path, &group.instances)
                .len(),
            3
        );
        assert!(session.delete_issue_instances(id, &group.path, &group.instances));
        assert_eq!(session.map_issues(id)[0].instances, [group.instances[1]]);
        assert!(session.undo());
        assert_eq!(session.map_issues(id)[0].instances, group.instances);
        assert!(!issues::has_replacement_kind(
            session.tree().unwrap(),
            &TreePath::parse("/missing_root/type")
        ));
    }

    #[test]
    fn remote_changes_and_codebase_reload_refresh_groups_without_stale_ids() {
        let mut session = unknown_paths_session();
        let id = id_for(&session);
        session.take_loaded_issues();
        let source = TreePath::parse("/obj/removed_table");
        let groups = session.map_issues(id);
        let removed = groups[0].instances[0];
        session.apply_remote_tiles(
            id,
            vec![(
                Coord::new(1, 1, 1),
                vec![Prefab::new(TreePath::parse("/turf/open/floor"))],
            )],
        );
        assert_eq!(session.map_issues(id)[0].instances.len(), 3);
        assert!(!session.map_issues(id)[0].instances.contains(&removed));
        assert!(
            session.take_loaded_issues().is_none(),
            "ordinary edits do not request panel focus"
        );
        let mut loaded =
            crate::loader::load_codebase(&examples().join("test.dme"), &Default::default(), &Progress::new()).unwrap();
        loaded.environment.tree.register(&source, Location::default());
        session.apply_codebase(loaded);
        assert_eq!(session.map_issues(id).len(), 1);
        assert_eq!(session.map_issues(id)[0].path.to_string(), "/turf/removed_floor");
        assert!(session.take_loaded_issues().is_some());
        session.load_environment(&examples().join("test.dme")).unwrap();
        assert_eq!(session.map_issues(id)[0].instances.len(), 3);
    }
}
