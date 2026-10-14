use std::{collections::HashMap, sync::Arc};

use dmm::{Coord, Prefab, PrefabInstanceId, Size, Tile};

use crate::{
    document::{PrefabInstance, PrefabLocation},
    grid::Grid,
};

#[derive(Debug, Default, Clone)]
pub(super) struct OverlayTiles {
    pub cells: HashMap<Coord, Arc<(Vec<PrefabInstanceId>, Tile)>>,
    locations: HashMap<PrefabInstanceId, PrefabLocation>,
}

impl OverlayTiles {
    pub fn remove(&mut self, coord: Coord) {
        if let Some(cell) = self.cells.remove(&coord) {
            for id in &cell.0 {
                if self.locations.get(id).is_some_and(|location| location.coord == coord) {
                    self.locations.remove(id);
                }
            }
        }
    }

    pub fn replace(&mut self, coord: Coord, entries: Vec<(PrefabInstanceId, Prefab)>) {
        self.remove(coord);
        let (ids, tile) = entries.into_iter().unzip::<_, _, Vec<_>, Vec<_>>();
        for (prefab_index, id) in ids.iter().enumerate() {
            self.locations.insert(*id, PrefabLocation { coord, prefab_index });
        }

        self.cells.insert(coord, Arc::new((ids, tile)));
    }
}

#[derive(Clone, Copy)]
pub struct MapView<'a> {
    source: &'a Grid,
    tiles: Option<&'a OverlayTiles>,
}

impl<'a> From<&'a Grid> for MapView<'a> {
    fn from(source: &'a Grid) -> Self { Self { source, tiles: None } }
}

impl<'a> MapView<'a> {
    pub(super) fn new(source: &'a Grid, tiles: &'a OverlayTiles) -> Self {
        Self {
            source,
            tiles: Some(tiles),
        }
    }

    pub fn size(self) -> Size { self.source.size() }

    pub fn tile_at(self, coord: Coord) -> Option<&'a Tile> {
        self.source.tile_at(coord)?;

        self.tiles
            .and_then(|tiles| tiles.cells.get(&coord))
            .map(|cell| &cell.1)
            .or_else(|| self.source.tile_at(coord))
    }

    pub fn ids_at(self, coord: Coord) -> &'a [PrefabInstanceId] {
        if self.source.tile_at(coord).is_none() {
            return &[];
        }

        self.tiles
            .and_then(|tiles| tiles.cells.get(&coord))
            .map_or_else(|| self.source.ids_at(coord), |cell| cell.0.as_slice())
    }

    pub fn location(self, id: PrefabInstanceId) -> Option<PrefabLocation> {
        if let Some(location) = self.tiles.and_then(|tiles| tiles.locations.get(&id)).copied() {
            return self.source.tile_at(location.coord).map(|_| location);
        }

        let location = self.source.location(id)?;
        (!self
            .tiles
            .is_some_and(|tiles| tiles.cells.contains_key(&location.coord)))
        .then_some(location)
    }

    pub fn prefab_instance(self, id: PrefabInstanceId) -> Option<PrefabInstance<'a>> {
        let location = self.location(id)?;
        let prefab = self.tile_at(location.coord)?.get(location.prefab_index)?;

        Some(PrefabInstance::new(id, prefab, location))
    }

    pub fn prefab_instances(self) -> impl Iterator<Item = PrefabInstance<'a>> {
        self.source
            .prefab_instances()
            .filter(move |instance| {
                !self
                    .tiles
                    .is_some_and(|tiles| tiles.cells.contains_key(&instance.location.coord))
            })
            .chain(
                self.tiles
                    .into_iter()
                    .flat_map(|tiles| tiles.locations.keys())
                    .filter_map(move |id| self.prefab_instance(*id)),
            )
    }
}
