pub(crate) mod panel;
pub(crate) mod session;

use base::coord::Coord;
use dmm::{Map, Size};

use crate::{
    clipboard::TileRegion,
    document::{DocumentId, MapDocument, selection::Selection},
    frame::options::HiddenTypes,
    tool::SelectionRotation,
};

struct StashEntry {
    document: MapDocument,
    hidden: HiddenTypes,
}

impl StashEntry {
    fn new(region: TileRegion) -> Self {
        let (width, height) = (region.width(), region.height());
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });

        let bounds = Selection {
            min: Coord::new(1, 1, 1),
            max: Coord::new(width, height, 1),
        };

        for (coord, tile) in region.destinations(bounds, SelectionRotation::Original) {
            let key = map.intern_tile(tile.clone());
            map.grid[0][(height - coord.y) as usize][(coord.x - 1) as usize] = key;
        }

        Self {
            document: MapDocument::stash(map),
            hidden: region.hidden().clone(),
        }
    }
}

#[derive(Default)]
pub struct Stash {
    entries: Vec<StashEntry>,
}

impl Stash {
    pub fn push(&mut self, region: TileRegion) -> DocumentId {
        let entry = StashEntry::new(region);
        let id = entry.document.id();
        self.entries.push(entry);

        id
    }

    pub fn remove(&mut self, id: DocumentId) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.document.id() == id) else {
            return false;
        };

        self.entries.remove(index);

        true
    }

    pub fn ids(&self) -> Vec<DocumentId> {
        self.entries.iter().map(|entry| entry.document.id()).collect()
    }

    pub fn document(&self, id: DocumentId) -> Option<&MapDocument> {
        self.entry(id).map(|entry| &entry.document)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut MapDocument> {
        self.entries
            .iter_mut()
            .find(|entry| entry.document.id() == id)
            .map(|entry| &mut entry.document)
    }

    pub fn hidden(&self, id: DocumentId) -> Option<&HiddenTypes> {
        self.entry(id).map(|entry| &entry.hidden)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn entry(&self, id: DocumentId) -> Option<&StashEntry> {
        self.entries.iter().find(|entry| entry.document.id() == id)
    }
}

#[cfg(test)]
mod tests;
