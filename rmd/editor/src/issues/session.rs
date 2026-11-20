use base::{coord::Coord, path::TreePath, types::Identifier};
use prefab::PrefabInstanceId;
use std::{cell::Ref, collections::BTreeMap, sync::Arc};

use crate::session::Session;
use crate::{
    document::{DocumentId, MapDocument, instance::VarMutation},
    frame::{InstanceIssue, TileIssues},
    issues,
    tool::place::{is_area, is_missing_base},
};

#[derive(Debug)]
pub(crate) struct MapIssueGroup {
    pub path: TreePath,
    pub instances: Vec<PrefabInstanceId>,
}

#[derive(Debug)]
pub(crate) struct UndefinedVarGroup {
    pub name: Identifier,
    pub instances: Vec<PrefabInstanceId>,
}

#[derive(Default)]
pub(crate) struct IssueGroups {
    key: (u64, u64),
    groups: Arc<[MapIssueGroup]>,
    missing_bases: Arc<[Coord]>,
    multiple_areas: Arc<[PrefabInstanceId]>,
    undefined_vars: Arc<[UndefinedVarGroup]>,
}

fn instance_sort_key(document: &MapDocument, id: PrefabInstanceId) -> (u32, u32, u32, usize, u64) {
    let location = document.instance_location(id).unwrap();

    (
        location.coord.z,
        location.coord.y,
        location.coord.x,
        location.prefab_index,
        id.get(),
    )
}

impl Session {
    fn issues(&self, id: DocumentId) -> Option<Ref<'_, IssueGroups>> {
        let (document, cache) = (self.state.document(id)?, self.caches.get(&id)?);
        let key = (document.generation(), cache.revision);
        let mut issues = cache.issues.borrow_mut();
        if issues.key != key {
            let mut grouped = BTreeMap::<String, MapIssueGroup>::new();
            let mut undefined = BTreeMap::<Identifier, Vec<PrefabInstanceId>>::new();
            for (id, found) in cache.instances.instance_issues() {
                let Some(instance) = document.prefab_instance(id) else {
                    continue;
                };

                for issue in found {
                    match issue {
                        InstanceIssue::UnknownType => {
                            let path = &instance.prefab().path;
                            grouped
                                .entry(path.to_string())
                                .or_insert_with(|| MapIssueGroup {
                                    path: path.clone(),
                                    instances: Vec::new(),
                                })
                                .instances
                                .push(id);
                        },
                        InstanceIssue::UndefinedVar(name) => undefined.entry(name.clone()).or_default().push(id),
                    }
                }
            }

            for group in grouped.values_mut() {
                group
                    .instances
                    .sort_unstable_by_key(|id| instance_sort_key(document, *id));
            }

            issues.groups = grouped.into_values().collect::<Vec<_>>().into();

            let mut missing = Vec::new();
            let mut areas = Vec::new();
            for (coord, found) in cache.instances.tile_issues() {
                if found.contains(TileIssues::MISSING_BASE) {
                    missing.push(coord);
                }

                if found.contains(TileIssues::MULTIPLE_AREAS) {
                    areas.push(coord);
                }
            }

            missing.retain(|coord| document.map.tile_at(*coord).is_some_and(|tile| is_missing_base(tile)));
            missing.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
            issues.missing_bases = missing.into();

            areas.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
            let mut multiple_areas = Vec::new();
            for coord in areas {
                let found = document
                    .instance_ids_at(coord)
                    .iter()
                    .copied()
                    .filter(|id| {
                        document
                            .prefab_instance(*id)
                            .is_some_and(|instance| is_area(instance.prefab()))
                    })
                    .collect::<Vec<_>>();
                if found.len() > 1 {
                    multiple_areas.extend(found);
                }
            }

            issues.multiple_areas = multiple_areas.into();

            issues.undefined_vars = undefined
                .into_iter()
                .map(|(name, mut instances)| {
                    instances.sort_unstable_by_key(|id| instance_sort_key(document, *id));
                    UndefinedVarGroup { name, instances }
                })
                .collect::<Vec<_>>()
                .into();
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

    pub(crate) fn multiple_area_instances(&self, id: DocumentId) -> Arc<[PrefabInstanceId]> {
        self.issues(id)
            .map(|issues| Arc::clone(&issues.multiple_areas))
            .unwrap_or_default()
    }

    pub(crate) fn undefined_vars(&self, id: DocumentId) -> Arc<[UndefinedVarGroup]> {
        self.issues(id)
            .map(|issues| Arc::clone(&issues.undefined_vars))
            .unwrap_or_default()
    }

    pub(crate) fn has_tile_issues(&self, id: DocumentId) -> bool {
        !self.multiple_area_instances(id).is_empty() || !self.undefined_vars(id).is_empty()
    }

    pub(crate) fn issue_count(&self, id: DocumentId) -> usize {
        self.map_issues(id)
            .iter()
            .map(|group| group.instances.len())
            .sum::<usize>()
            + self.missing_base_tiles(id).len()
            + self.multiple_area_instances(id).len()
            + self
                .undefined_vars(id)
                .iter()
                .map(|group| group.instances.len())
                .sum::<usize>()
    }

    pub(crate) fn note_loaded_issues(&mut self, id: DocumentId) {
        if self.issue_count(id) > 0 {
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

    pub(crate) fn remove_issue_var(
        &mut self, id: DocumentId, instances: &[PrefabInstanceId], name: &Identifier,
    ) -> bool {
        let Some(document) = self.state.document(id) else {
            return false;
        };

        let editable = instances
            .iter()
            .copied()
            .filter(|instance| {
                document
                    .instance_location(*instance)
                    .is_some_and(|location| document.allows_edit_at(location.coord))
            })
            .collect::<Vec<_>>();
        if editable.is_empty() || !self.set_active_document(id) {
            return false;
        }

        let changed = self.state.active_document_mut().and_then(|document| {
            document.edit_instances(
                &editable,
                format!("remove {name}"),
                None,
                &[VarMutation::Remove(name.clone())],
                None,
            )
        });
        if changed != Some(true) {
            return false;
        }

        self.update_instances(&editable);

        true
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
