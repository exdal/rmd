use dmm::{Coord, Map, Size, Tile};

use crate::{
    clipboard::TileRegion,
    document::{DocumentId, MapDocument, grid::Grid, selection::Selection},
    frame::options::HiddenTypes,
    tool::SelectionRotation,
};

const GAP: u32 = 2;

struct StashEntry {
    region: TileRegion,
    x: u32,
}

impl StashEntry {
    fn bounds(&self) -> Selection {
        Selection {
            min: Coord::new(self.x, 1, 1),
            max: Coord::new(self.x + self.region.width() - 1, self.region.height(), 1),
        }
    }
}

pub struct Stash {
    document: MapDocument,
    entries: Vec<StashEntry>,
}

impl Default for Stash {
    fn default() -> Self {
        Self::new()
    }
}

impl Stash {
    pub fn new() -> Self {
        Self {
            document: MapDocument::stash(Map::new(Size { x: 1, y: 1, z: 1 })),
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, region: TileRegion) {
        let x = self.entries.last().map_or(1, |last| last.bounds().max.x + 1 + GAP);
        self.entries.push(StashEntry { region, x });
        self.rebuild();
    }

    pub fn remove_at(&mut self, coord: Coord) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.bounds().contains(coord)) else {
            return false;
        };

        self.entries.remove(index);
        self.repack();
        self.rebuild();

        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.rebuild();
    }

    pub fn entry_bounds(&self, coord: Coord) -> Option<Selection> {
        self.entries
            .iter()
            .map(StashEntry::bounds)
            .find(|bounds| bounds.contains(coord))
    }

    pub fn entry_hidden(&self, coord: Coord) -> Option<&HiddenTypes> {
        self.entries
            .iter()
            .find(|entry| entry.bounds().contains(coord))
            .map(|entry| entry.region.hidden())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn id(&self) -> DocumentId {
        self.document.id()
    }

    pub fn document(&self) -> &MapDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut MapDocument {
        &mut self.document
    }

    fn repack(&mut self) {
        let mut x = 1;
        for entry in &mut self.entries {
            entry.x = x;
            x += entry.region.width() + GAP;
        }
    }

    fn rebuild(&mut self) {
        let width = self.entries.last().map_or(1, |last| last.bounds().max.x);
        let height = self
            .entries
            .iter()
            .map(|entry| entry.region.height())
            .max()
            .unwrap_or(1);
        let mut tiles = vec![Tile::new(); width as usize * height as usize];

        for entry in &self.entries {
            let destinations = entry.region.destinations(entry.bounds(), SelectionRotation::Original);
            for (coord, tile) in destinations {
                tiles[((coord.y - 1) * width + coord.x - 1) as usize] = tile.clone();
            }
        }

        self.document.replace_grid(Grid::from_level_tiles(width, height, tiles));
    }
}

#[cfg(test)]
mod tests;
