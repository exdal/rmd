use core::{bitset::BitIndex, interner::SymbolMap};
use std::collections::HashMap;

use objtree::{SharedVarId, TypeId};

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
    statics: HashMap<Static, Option<GenericValue>>,
}

/// storage every call of a proc, or every instance of a type, shares
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Static {
    /// `var/static` inside a proc
    Local(u32),
    /// `var/static` or `var/global` on a type
    Var(SharedVarId),
}

impl Static {
    fn index(self) -> usize {
        match self {
            Self::Local(id) => id as usize,
            Self::Var(id) => id.0 as usize,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(pub u32);

impl BitIndex for ObjectId {
    fn bit_index(self) -> usize {
        self.0 as usize
    }
}

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
    /// `None` until the declaration runs or the initializer is called
    static_locals: Vec<Option<GenericValue>>,
    shared_vars: Vec<Option<GenericValue>>,
    journal: Option<Journal>,
}

impl Heap {
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(id.0 as usize).filter(|o| !o.deleted)
    }

    pub fn list(&self, id: ListId) -> Option<&ListData> {
        self.lists.get(id.0 as usize)
    }

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

    pub fn static_value(&self, key: Static) -> Option<&GenericValue> {
        let values = match key {
            Static::Local(_) => &self.static_locals,
            Static::Var(_) => &self.shared_vars,
        };

        values.get(key.index())?.as_ref()
    }

    pub fn static_value_mut(&mut self, key: Static) -> &mut Option<GenericValue> {
        let values = match key {
            Static::Local(_) => &mut self.static_locals,
            Static::Var(_) => &mut self.shared_vars,
        };
        let index = key.index();
        if values.len() <= index {
            values.resize(index + 1, None);
        }

        let value = &mut values[index];
        if let Some(j) = &mut self.journal {
            j.statics.entry(key).or_insert_with(|| value.clone());
        }

        value
    }

    pub fn begin(&mut self) {
        self.journal = Some(Journal {
            objects_len: self.objects.len(),
            lists_len: self.lists.len(),
            objects: HashMap::new(),
            lists: HashMap::new(),
            statics: HashMap::new(),
        });
    }

    pub fn before_object(&self, id: ObjectId) -> Option<&Object> {
        self.journal.as_ref()?.objects.get(&id).or_else(|| self.object(id))
    }

    pub fn before_list(&self, id: ListId) -> Option<&ListData> {
        self.journal.as_ref()?.lists.get(&id).or_else(|| self.list(id))
    }

    pub fn list_changed(&self, id: ListId) -> bool {
        self.journal.as_ref().is_some_and(|j| j.lists.contains_key(&id))
    }

    pub fn changed_objects(&self) -> Vec<ObjectId> {
        self.journal
            .as_ref()
            .map(|j| j.objects.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn changed(&self) -> bool {
        self.journal.as_ref().is_some_and(|j| {
            !j.objects.is_empty()
                || !j.lists.is_empty()
                || !j.statics.is_empty()
                || self.objects.len() > j.objects_len
                || self.lists.len() > j.lists_len
        })
    }

    pub fn commit(&mut self) {
        self.journal = None;
    }

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

        for (key, value) in j.statics {
            let values = match key {
                Static::Local(_) => &mut self.static_locals,
                Static::Var(_) => &mut self.shared_vars,
            };
            if let Some(slot) = values.get_mut(key.index()) {
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

    /// Deletes descendants created by runtime code, while leaving map placements to be relinked.
    pub(crate) fn delete_unplaced_contents(&mut self, parent: ObjectId) -> Result<(), FaultKind> {
        let mut pending = self
            .object(parent)
            .map(|object| object.contents.clone())
            .unwrap_or_default();

        while let Some(id) = pending.pop() {
            let Some(object) = self.object(id) else {
                continue;
            };
            if object.instance.is_some() {
                continue;
            }

            pending.extend(object.contents.iter().copied());
            self.relocate(id, None)?;
            self.object_mut(id)?.deleted = true;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use objtree::SharedVarId;

    use super::{Heap, Static};
    use crate::value::GenericValue;

    #[test]
    fn rollback_restores_statics_written_in_the_journal() {
        let mut heap = Heap::default();
        *heap.static_value_mut(Static::Local(0)) = Some(GenericValue::Num(1.0));
        heap.begin();
        *heap.static_value_mut(Static::Local(0)) = Some(GenericValue::Num(2.0));
        *heap.static_value_mut(Static::Var(SharedVarId(0))) = Some(GenericValue::Num(3.0));

        assert!(heap.changed());
        heap.rollback();
        assert_eq!(heap.static_value(Static::Local(0)), Some(&GenericValue::Num(1.0)));
        assert_eq!(heap.static_value(Static::Var(SharedVarId(0))), None);
    }
}
