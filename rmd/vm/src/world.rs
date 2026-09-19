use std::{collections::HashMap, path::PathBuf};

use crate::heap::{Heap, ObjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Position {
    pub fn new(x: i32, y: i32, z: i32) -> Self { Self { x, y, z } }

    pub fn step(self, dir: i32) -> Self {
        Self {
            x: self.x.saturating_add(i32::from(dir & 4 != 0) - i32::from(dir & 8 != 0)),
            y: self.y.saturating_add(i32::from(dir & 1 != 0) - i32::from(dir & 2 != 0)),
            z: self.z,
        }
    }
}

#[derive(Debug, Default)]
pub struct World {
    pub turfs: HashMap<Position, ObjectId>,
    pub areas: HashMap<Position, ObjectId>,
    pub size: [i32; 3],
    pub root: Option<PathBuf>,
}

impl World {
    pub fn turf_at(&self, pos: Position) -> Option<ObjectId> { self.turfs.get(&pos).copied() }

    pub fn position(&self, heap: &Heap, id: ObjectId) -> Option<Position> {
        let mut next = Some(id);
        for _ in 0..=heap.objects.len() {
            let object = heap.object(next?)?;
            if let Some(pos) = object.position {
                return Some(pos);
            }
            next = object.loc;
        }
        None
    }

    pub fn get_step(&self, heap: &Heap, id: ObjectId, dir: i32) -> Option<ObjectId> {
        self.turf_at(self.position(heap, id)?.step(dir))
    }

    pub fn area_of(&self, heap: &Heap, id: ObjectId) -> Option<ObjectId> {
        self.areas.get(&self.position(heap, id)?).copied()
    }
}
