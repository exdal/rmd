use std::{fmt, iter::FusedIterator, ops::Index, slice};

use dmm::Tile;
use prefab::{Prefab, PrefabId, table::PrefabTable};

#[derive(Clone, Copy)]
enum Source<'a> {
    Prefabs(&'a [Prefab]),
    Interned {
        table: &'a PrefabTable,
        ids: &'a [PrefabId],
    },
}

#[derive(Clone, Copy)]
pub struct TileView<'a> {
    source: Source<'a>,
}

impl<'a> TileView<'a> {
    pub(super) fn interned(table: &'a PrefabTable, ids: &'a [PrefabId]) -> Self {
        Self {
            source: Source::Interned { table, ids },
        }
    }

    pub fn len(self) -> usize {
        match self.source {
            Source::Prefabs(prefabs) => prefabs.len(),
            Source::Interned { ids, .. } => ids.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn get(self, index: usize) -> Option<&'a Prefab> {
        match self.source {
            Source::Prefabs(prefabs) => prefabs.get(index),
            Source::Interned { table, ids } => ids.get(index).map(|id| table.get(*id)),
        }
    }

    pub fn first(self) -> Option<&'a Prefab> {
        self.get(0)
    }

    pub fn contains(self, prefab: &Prefab) -> bool {
        self.iter().any(|candidate| candidate == prefab)
    }

    pub fn iter(self) -> Iter<'a> {
        Iter {
            source: match self.source {
                Source::Prefabs(prefabs) => IterSource::Prefabs(prefabs.iter()),
                Source::Interned { table, ids } => IterSource::Interned { table, ids: ids.iter() },
            },
        }
    }

    pub fn to_tile(self) -> Tile {
        self.iter().cloned().collect()
    }
}

impl Default for TileView<'_> {
    fn default() -> Self {
        Self {
            source: Source::Prefabs(&[]),
        }
    }
}

impl fmt::Debug for TileView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a> From<&'a Tile> for TileView<'a> {
    fn from(tile: &'a Tile) -> Self {
        Self {
            source: Source::Prefabs(tile),
        }
    }
}

impl<'a> IntoIterator for TileView<'a> {
    type IntoIter = Iter<'a>;
    type Item = &'a Prefab;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Index<usize> for TileView<'_> {
    type Output = Prefab;

    fn index(&self, index: usize) -> &Prefab {
        self.get(index).expect("tile index out of bounds")
    }
}

impl PartialEq for TileView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl PartialEq<Tile> for TileView<'_> {
    fn eq(&self, other: &Tile) -> bool {
        self.iter().eq(other)
    }
}

#[derive(Clone)]
enum IterSource<'a> {
    Prefabs(slice::Iter<'a, Prefab>),
    Interned {
        table: &'a PrefabTable,
        ids: slice::Iter<'a, PrefabId>,
    },
}

#[derive(Clone)]
pub struct Iter<'a> {
    source: IterSource<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Prefab;

    fn next(&mut self) -> Option<&'a Prefab> {
        match &mut self.source {
            IterSource::Prefabs(prefabs) => prefabs.next(),
            IterSource::Interned { table, ids } => ids.next().map(|id| table.get(*id)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.source {
            IterSource::Prefabs(prefabs) => prefabs.size_hint(),
            IterSource::Interned { ids, .. } => ids.size_hint(),
        }
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.source {
            IterSource::Prefabs(prefabs) => prefabs.next_back(),
            IterSource::Interned { table, ids } => ids.next_back().map(|id| table.get(*id)),
        }
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl FusedIterator for Iter<'_> {}
