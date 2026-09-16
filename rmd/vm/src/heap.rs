use core::interner::SymbolMap;
use std::collections::HashMap;

use objtree::TypeId;

use crate::{
    FaultKind,
    value::{GenericValue, ListData, ListId},
    world::Position,
};

#[derive(Debug)]
struct Journal {
    objects_len: usize,
    lists_len: usize,
    objects: HashMap<ObjectId, Object>,
    lists: HashMap<ListId, ListData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(pub u32);

#[derive(Debug, Clone)]
pub struct Object {
    pub ty: TypeId,
    pub vars: SymbolMap<GenericValue>,
    pub loc: Option<ObjectId>,
    pub contents: Vec<ObjectId>,
    pub position: Option<Position>,
    pub instance: Option<u64>,
    pub deleted: bool,
}

impl Object {
    pub fn new(ty: TypeId) -> Self {
        Self {
            ty,
            vars: SymbolMap::default(),
            loc: None,
            contents: Vec::new(),
            position: None,
            instance: None,
            deleted: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct Heap {
    pub(crate) objects: Vec<Object>,
    pub(crate) lists: Vec<ListData>,
    journal: Option<Journal>,
}

impl Heap {
    pub fn object(&self, id: ObjectId) -> Option<&Object> { self.objects.get(id.0 as usize).filter(|o| !o.deleted) }

    pub fn list(&self, id: ListId) -> Option<&ListData> { self.lists.get(id.0 as usize) }

    pub fn objects(&self) -> impl Iterator<Item = (ObjectId, &Object)> {
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, o)| !o.deleted)
            .map(|(i, o)| (ObjectId(i as u32), o))
    }

    pub fn alloc_object(&mut self, object: Object) -> Result<ObjectId, FaultKind> {
        let id = ObjectId(u32::try_from(self.objects.len()).map_err(|_| FaultKind::Memory)?);
        self.objects.push(object);
        Ok(id)
    }

    pub fn alloc_list(&mut self, mut list: ListData) -> Result<ListId, FaultKind> {
        let id = ListId(u32::try_from(self.lists.len()).map_err(|_| FaultKind::Memory)?);
        list.reindex();
        self.lists.push(list);
        Ok(id)
    }

    pub fn object_mut(&mut self, id: ObjectId) -> Result<&mut Object, FaultKind> {
        let value = self.objects.get_mut(id.0 as usize).ok_or(FaultKind::InvalidReference)?;
        if let Some(j) = &mut self.journal
            && (id.0 as usize) < j.objects_len
        {
            j.objects.entry(id).or_insert_with(|| value.clone());
        }
        Ok(value)
    }

    pub fn list_mut(&mut self, id: ListId) -> Result<&mut ListData, FaultKind> {
        let value = self.lists.get_mut(id.0 as usize).ok_or(FaultKind::InvalidReference)?;
        if let Some(j) = &mut self.journal
            && (id.0 as usize) < j.lists_len
        {
            j.lists.entry(id).or_insert_with(|| value.clone());
        }
        Ok(value)
    }

    pub fn begin(&mut self) {
        self.journal = Some(Journal {
            objects_len: self.objects.len(),
            lists_len: self.lists.len(),
            objects: HashMap::new(),
            lists: HashMap::new(),
        });
    }

    pub fn before_object(&self, id: ObjectId) -> Option<&Object> {
        self.journal.as_ref()?.objects.get(&id).or_else(|| self.object(id))
    }

    pub fn before_list(&self, id: ListId) -> Option<&ListData> {
        self.journal.as_ref()?.lists.get(&id).or_else(|| self.list(id))
    }

    pub fn list_changed(&self, id: ListId) -> bool { self.journal.as_ref().is_some_and(|j| j.lists.contains_key(&id)) }

    pub fn changed_objects(&self) -> Vec<ObjectId> {
        self.journal
            .as_ref()
            .map(|j| j.objects.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn commit(&mut self) { self.journal = None; }

    pub fn rollback(&mut self) {
        let Some(j) = self.journal.take() else {
            return;
        };

        self.objects.truncate(j.objects_len);
        self.lists.truncate(j.lists_len);

        for (id, value) in j.objects {
            if let Some(slot) = self.objects.get_mut(id.0 as usize) {
                *slot = value;
            }
        }

        for (id, value) in j.lists {
            if let Some(slot) = self.lists.get_mut(id.0 as usize) {
                *slot = value;
            }
        }
    }

    pub fn relocate(&mut self, id: ObjectId, loc: Option<ObjectId>) -> Result<(), FaultKind> {
        let mut ancestor = loc;
        for _ in 0..=self.objects.len() {
            if ancestor == Some(id) {
                return Err(FaultKind::InvalidOperation("cyclic loc".into()));
            }
            ancestor = ancestor.and_then(|id| self.object(id)).and_then(|o| o.loc);
            if ancestor.is_none() {
                break;
            }
        }

        let old = self.object(id).and_then(|o| o.loc);

        if old == loc {
            return Ok(());
        }

        if let Some(old) = old {
            self.object_mut(old)?.contents.retain(|child| *child != id);
        }

        self.object_mut(id)?.loc = loc;
        if let Some(loc) = loc {
            self.object_mut(loc)?.contents.push(id);
        }

        Ok(())
    }
}
