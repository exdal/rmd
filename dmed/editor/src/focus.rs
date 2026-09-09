use std::collections::HashSet;

use dmm::{Coord, Prefab, PrefabInstanceId};

use crate::command::Edit;

#[derive(Debug, Clone, PartialEq)]
pub struct AreaFocus {
    seed: Coord,
    prefab: Prefab,
    component: PrefabInstanceId,
    tiles: HashSet<Coord>,
}

impl AreaFocus {
    pub fn new(seed: Coord, prefab: Prefab, component: PrefabInstanceId, tiles: HashSet<Coord>) -> Self {
        Self {
            seed,
            prefab,
            component,
            tiles,
        }
    }

    pub fn seed(&self) -> Coord { self.seed }

    pub fn prefab(&self) -> &Prefab { &self.prefab }

    pub fn component(&self) -> PrefabInstanceId { self.component }

    pub fn allows(&self, coord: Coord) -> bool { self.tiles.contains(&coord) }

    pub fn allows_edit(&self, edit: &Edit) -> bool { edit.changes.iter().all(|change| self.allows(change.coord)) }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Prefab, PrefabInstanceId};

    use super::AreaFocus;
    use crate::command::{Edit, TileChange};

    fn focus(tiles: &[Coord]) -> AreaFocus {
        AreaFocus::new(
            tiles.first().copied().unwrap_or(Coord::new(1, 1, 1)),
            Prefab::new(TreePath::parse("/area/station")),
            PrefabInstanceId::from_raw(1).unwrap(),
            tiles.iter().copied().collect(),
        )
    }

    fn edit(coords: &[Coord]) -> Edit {
        let mut edit = Edit::new("test");
        edit.changes = coords
            .iter()
            .map(|coord| TileChange {
                coord: *coord,
                before: Vec::new(),
                after: Vec::new(),
            })
            .collect();

        edit
    }

    #[test]
    fn an_edit_reaching_outside_the_region_is_refused_whole() {
        let inside = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        let outside = Coord::new(4, 1, 1);
        let focus = focus(&[inside, neighbor]);

        assert!(focus.allows(inside));
        assert!(!focus.allows(outside));
        assert!(!focus.allows(Coord::new(1, 1, 2)));

        assert!(focus.allows_edit(&edit(&[inside, neighbor])));
        assert!(!focus.allows_edit(&edit(&[inside, outside])));
        assert!(focus.allows_edit(&edit(&[])));
    }
}
