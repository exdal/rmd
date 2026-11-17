use std::{ops::Index, slice};

use dmm::{Prefab, Tile};

pub type Iter<'a> = slice::Iter<'a, Prefab>;

#[derive(Debug, Clone, Copy, Default)]
pub struct TileView<'a> {
    prefabs: &'a [Prefab],
}

impl<'a> TileView<'a> {
    pub fn len(self) -> usize {
        self.prefabs.len()
    }

    pub fn is_empty(self) -> bool {
        self.prefabs.is_empty()
    }

    pub fn get(self, index: usize) -> Option<&'a Prefab> {
        self.prefabs.get(index)
    }

    pub fn first(self) -> Option<&'a Prefab> {
        self.prefabs.first()
    }

    pub fn contains(self, prefab: &Prefab) -> bool {
        self.prefabs.contains(prefab)
    }

    pub fn iter(self) -> Iter<'a> {
        self.prefabs.iter()
    }

    pub fn to_tile(self) -> Tile {
        self.prefabs.to_vec()
    }
}

impl<'a> From<&'a Tile> for TileView<'a> {
    fn from(tile: &'a Tile) -> Self {
        Self { prefabs: tile }
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
        &self.prefabs[index]
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
