use base::{coord::Coord, path::TreePath};
use prefab::PrefabInstanceId;
use std::{cell::Ref, collections::BTreeMap, sync::Arc};

use crate::session::Session;
use crate::{document::DocumentId, issues};

#[derive(Debug)]
pub(crate) struct MapIssueGroup {
    pub path: TreePath,
    pub instances: Vec<PrefabInstanceId>,
}

#[derive(Default)]
pub(crate) struct IssueGroups {
    key: (u64, u64),
    groups: Arc<[MapIssueGroup]>,
    missing_bases: Arc<[Coord]>,
}

impl Session {
    fn issues(&self, id: DocumentId) -> Option<Ref<'_, IssueGroups>> {
        let (document, cache) = (self.state.document(id)?, self.caches.get(&id)?);
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
            issues.missing_bases = issues::missing_base_tiles(document).into();
            issues.key = key;
        }

        drop(issues);

        Some(cache.issues.borrow())
    }

    pub(crate) fn map_issues(&self, id: DocumentId) -> Arc<[MapIssueGroup]> {
        self.issues(id)
            .map(|issues| Arc::clone(&issues.groups))
            .unwrap_or_default()
    }

    pub(crate) fn missing_base_tiles(&self, id: DocumentId) -> Arc<[Coord]> {
        self.issues(id)
            .map(|issues| Arc::clone(&issues.missing_bases))
            .unwrap_or_default()
    }

    pub(crate) fn note_loaded_issues(&mut self, id: DocumentId) {
        if !self.map_issues(id).is_empty() || !self.missing_base_tiles(id).is_empty() {
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
        let action = self.state.pair_mut(id).and_then(|(environment, document)| {
            issues::delete_instances(document, &environment.tree, &environment.world, path, instances)
        });
        action.is_some_and(|action| self.set_active_document(id) && self.commit(action, None))
    }

    pub(crate) fn fill_missing_bases(&mut self, id: DocumentId, tiles: &[Coord]) -> bool {
        let action = self.state.pair_mut(id).and_then(|(environment, document)| {
            issues::fill_missing_bases(document, &environment.tree, &environment.world, tiles)
        });
        action.is_some_and(|action| self.set_active_document(id) && self.commit(action, None))
    }
}

#[cfg(test)]
mod tests;
