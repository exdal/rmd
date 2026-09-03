use std::path::PathBuf;

use dmm::{Coord, Map};

use crate::command::{Edit, History};

pub struct MapDocument {
    pub path: Option<PathBuf>,
    pub map: Map,
    pub history: History,
    pub z: u32,
    pub selection: Option<Selection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub min: Coord,
    pub max: Coord,
}

impl Selection {
    pub fn from_drag(anchor: Coord, cursor: Coord) -> Self {
        Self {
            min: Coord::new(anchor.x.min(cursor.x), anchor.y.min(cursor.y), anchor.z),
            max: Coord::new(anchor.x.max(cursor.x), anchor.y.max(cursor.y), anchor.z),
        }
    }

    pub fn contains(&self, coord: Coord) -> bool {
        coord.z == self.min.z
            && (self.min.x..=self.max.x).contains(&coord.x)
            && (self.min.y..=self.max.y).contains(&coord.y)
    }

    pub fn iter(&self) -> impl Iterator<Item = Coord> + '_ {
        let z = self.min.z;

        (self.min.y..=self.max.y).flat_map(move |y| (self.min.x..=self.max.x).map(move |x| Coord::new(x, y, z)))
    }
}

impl MapDocument {
    pub fn new(map: Map, z: u32) -> Self {
        Self {
            path: None,
            map,
            history: History::new(),
            z,
            selection: None,
        }
    }

    pub fn open(path: impl Into<PathBuf>, map: Map, z: u32) -> Self {
        Self {
            path: Some(path.into()),
            ..Self::new(map, z)
        }
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());

        if self.is_dirty() { format!("{name} *") } else { name }
    }

    pub fn is_dirty(&self) -> bool { self.history.is_dirty() }

    pub fn apply(&mut self, edit: Edit) { self.history.apply(&mut self.map, edit); }

    pub fn undo(&mut self) -> bool { self.history.undo(&mut self.map).is_some() }

    pub fn redo(&mut self) -> bool { self.history.redo(&mut self.map).is_some() }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(std::io::Error::other("document has no path"));
        };

        self.map.prune_dictionary();
        dmm::writer::MapWriter::new(&self.map).save(path)?;
        self.history.mark_saved();

        Ok(())
    }

    pub fn clamp(&self, x: u32, y: u32) -> Coord {
        Coord::new(
            x.clamp(1, self.map.size.x.max(1)),
            y.clamp(1, self.map.size.y.max(1)),
            self.z,
        )
    }
}
