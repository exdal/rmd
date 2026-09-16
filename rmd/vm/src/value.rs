use core::{
    path::TreePath,
    types::{Identifier, ProcId},
};
use std::{collections::HashMap, sync::Arc};

use crate::heap::ObjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IteratorId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct RangeValue {
    pub start: f32,
    pub end: f32,
    pub step: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModifiedType {
    pub path: TreePath,
    pub overrides: Vec<(Identifier, GenericValue)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcRef {
    pub src: Receiver,
    pub proc: ProcId,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Receiver {
    #[default]
    None,
    Object(ObjectId),
    List(ListId),
}

impl Receiver {
    pub fn value(self) -> GenericValue {
        match self {
            Self::None => GenericValue::Null,
            Self::Object(id) => GenericValue::Object(id),
            Self::List(id) => GenericValue::List(id),
        }
    }

    pub fn object(self) -> Option<ObjectId> {
        match self {
            Self::Object(id) => Some(id),
            _ => None,
        }
    }

    pub fn list(self) -> Option<ListId> {
        match self {
            Self::List(id) => Some(id),
            _ => None,
        }
    }
}

impl From<Option<ObjectId>> for Receiver {
    fn from(id: Option<ObjectId>) -> Self { id.map_or(Self::None, Self::Object) }
}

#[derive(Debug, Clone, Default)]
pub enum GenericValue {
    #[default]
    Null,
    Num(f32),
    Text(Arc<str>),
    Resource(Arc<str>),
    Path(TreePath),
    Proc(ProcRef),
    List(ListId),
    Iterator(IteratorId),
    Range(RangeValue),
    ModifiedType(ModifiedType),
    ArgList(ListId),
    Object(ObjectId),
    World,
    Global,
    Omitted,
}

impl GenericValue {
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null | Self::Omitted => false,
            Self::Num(n) => *n != 0.0,
            Self::Text(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn num(&self) -> Option<f32> {
        match self {
            Self::Null => Some(0.0),
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) | Self::Resource(s) => Some(s),
            _ => None,
        }
    }

    pub fn object(&self) -> Option<ObjectId> {
        match self {
            Self::Object(id) => Some(*id),
            _ => None,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Null | Self::Omitted => String::new(),
            Self::Num(n) => n.to_string(),
            Self::Text(s) | Self::Resource(s) => s.to_string(),
            Self::Path(p) => p.to_string(),
            Self::List(_) => "/list".into(),
            Self::ArgList(_) => "/list".into(),
            Self::Iterator(_) => "[iterator]".into(),
            Self::Range(range) => format!("{} to {} step {}", range.start, range.end, range.step),
            Self::ModifiedType(value) => value.path.to_string(),
            Self::Object(id) => format!("[object {}]", id.0),
            Self::Proc(p) => format!("[proc {}]", p.proc.0),
            Self::World => "world".into(),
            Self::Global => "global".into(),
        }
    }
}

impl From<f32> for GenericValue {
    fn from(n: f32) -> Self { Self::Num(n) }
}

impl From<bool> for GenericValue {
    fn from(b: bool) -> Self { Self::Num(if b { 1.0 } else { 0.0 }) }
}

impl From<&str> for GenericValue {
    fn from(s: &str) -> Self { Self::Text(s.into()) }
}

impl From<String> for GenericValue {
    fn from(s: String) -> Self { Self::Text(s.into()) }
}

impl PartialEq for GenericValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null)
            | (Self::Omitted, Self::Omitted)
            | (Self::World, Self::World)
            | (Self::Global, Self::Global) => true,
            (Self::Num(a), Self::Num(b)) => a == b,
            (Self::Text(a), Self::Text(b)) | (Self::Resource(a), Self::Resource(b)) => a == b,
            (Self::Path(a), Self::Path(b)) => {
                a.segments == b.segments
                    && a.flags
                        .intersects(core::path::PathFlags::IS_PROC | core::path::PathFlags::IS_VERB)
                        == b.flags
                            .intersects(core::path::PathFlags::IS_PROC | core::path::PathFlags::IS_VERB)
            },
            (Self::List(a), Self::List(b)) => a == b,
            (Self::ArgList(a), Self::ArgList(b)) => a == b,
            (Self::Iterator(a), Self::Iterator(b)) => a == b,
            (Self::Range(a), Self::Range(b)) => a == b,
            (Self::ModifiedType(a), Self::ModifiedType(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Proc(a), Self::Proc(b)) => a == b,
            _ => false,
        }
    }
}

impl GenericValue {
    pub(crate) fn hash_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(self).hash(&mut hash);
        match self {
            Self::Num(n) => (if *n == 0.0 { 0 } else { n.to_bits() }).hash(&mut hash),
            Self::Text(s) | Self::Resource(s) => s.hash(&mut hash),
            Self::Path(p) => {
                p.segments.hash(&mut hash);
                p.flags
                    .intersects(core::path::PathFlags::IS_PROC | core::path::PathFlags::IS_VERB)
                    .hash(&mut hash);
            },
            Self::List(id) => id.hash(&mut hash),
            Self::ArgList(id) => id.hash(&mut hash),
            Self::Iterator(id) => id.hash(&mut hash),
            Self::Range(range) => {
                range.start.to_bits().hash(&mut hash);
                range.end.to_bits().hash(&mut hash);
                range.step.to_bits().hash(&mut hash);
            },
            Self::ModifiedType(value) => {
                value.path.segments.hash(&mut hash);
                for (name, value) in &value.overrides {
                    name.hash(&mut hash);
                    value.hash_key().hash(&mut hash);
                }
            },
            Self::Object(id) => id.hash(&mut hash),
            Self::Proc(p) => {
                p.src.hash(&mut hash);
                p.proc.hash(&mut hash);
            },
            _ => {},
        }
        hash.finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListKind {
    #[default]
    List,
    Alist,
}

#[derive(Debug, Clone, Default)]
pub struct ListData {
    pub entries: Vec<(GenericValue, Option<GenericValue>)>,
    pub kind: ListKind,
    index: HashMap<u64, Vec<usize>>,
}

impl PartialEq for ListData {
    fn eq(&self, other: &Self) -> bool { self.kind == other.kind && self.entries == other.entries }
}

impl ListData {
    pub fn new(entries: Vec<(GenericValue, Option<GenericValue>)>) -> Self {
        let mut list = Self::default();
        list.replace(entries);
        list
    }

    pub fn alist(entries: Vec<(GenericValue, Option<GenericValue>)>) -> Self {
        let mut list = Self {
            kind: ListKind::Alist,
            ..Self::default()
        };
        list.replace(entries);
        list
    }

    pub fn replace(&mut self, entries: Vec<(GenericValue, Option<GenericValue>)>) {
        self.entries = entries;
        self.reindex();
    }

    pub fn reindex(&mut self) {
        self.index.clear();
        for (i, (key, _)) in self.entries.iter().enumerate() {
            self.index.entry(key.hash_key()).or_default().push(i);
        }
    }

    pub fn push(&mut self, key: GenericValue, value: Option<GenericValue>) {
        self.index.entry(key.hash_key()).or_default().push(self.entries.len());
        self.entries.push((key, value));
    }

    pub fn position(&self, key: &GenericValue) -> Option<usize> {
        self.index
            .get(&key.hash_key())?
            .iter()
            .copied()
            .find(|i| self.entries.get(*i).is_some_and(|(k, _)| k == key))
    }

    pub fn get(&self, key: &GenericValue) -> GenericValue {
        if let GenericValue::Num(n) = key {
            return self
                .entries
                .get((*n as usize).wrapping_sub(1))
                .map(|(v, _)| v.clone())
                .unwrap_or_default();
        }
        self.position(key)
            .and_then(|i| self.entries.get(i))
            .and_then(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    pub fn contains(&self, value: &GenericValue) -> bool { self.position(value).is_some() }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppearanceDelta {
    pub vars: Vec<(Identifier, core::types::Value)>,
    pub overlays: Vec<AppearanceDelta>,
    pub underlays: Vec<AppearanceDelta>,
}

impl From<i32> for GenericValue {
    fn from(n: i32) -> Self { Self::Num(n as f32) }
}

impl From<usize> for GenericValue {
    fn from(n: usize) -> Self { Self::Num(n as f32) }
}
