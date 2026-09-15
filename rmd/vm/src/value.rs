use core::{
    path::TreePath,
    types::{Identifier, ProcId},
};
use std::{collections::HashMap, rc::Rc};

use crate::heap::ObjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct ProcRef {
    pub src: Option<ObjectId>,
    pub proc: ProcId,
}

#[derive(Debug, Clone, Default)]
pub enum RtValue {
    #[default]
    Null,
    Num(f32),
    Text(Rc<str>),
    Resource(Rc<str>),
    Path(TreePath),
    Proc(ProcRef),
    List(ListId),
    Object(ObjectId),
    World,
    Global,
    Omitted,
}

impl RtValue {
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
            Self::Object(id) => format!("[object {}]", id.0),
            Self::Proc(p) => format!("[proc {}]", p.proc.0),
            Self::World => "world".into(),
            Self::Global => "global".into(),
        }
    }
}

impl From<f32> for RtValue {
    fn from(n: f32) -> Self { Self::Num(n) }
}

impl From<bool> for RtValue {
    fn from(b: bool) -> Self { Self::Num(if b { 1.0 } else { 0.0 }) }
}

impl From<&str> for RtValue {
    fn from(s: &str) -> Self { Self::Text(s.into()) }
}

impl From<String> for RtValue {
    fn from(s: String) -> Self { Self::Text(s.into()) }
}

impl PartialEq for RtValue {
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
            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Proc(a), Self::Proc(b)) => a == b,
            _ => false,
        }
    }
}

impl RtValue {
    fn hash_key(&self) -> u64 {
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

#[derive(Debug, Clone, Default)]
pub struct RtList {
    pub entries: Vec<(RtValue, Option<RtValue>)>,
    index: HashMap<u64, Vec<usize>>,
}

impl PartialEq for RtList {
    fn eq(&self, other: &Self) -> bool { self.entries == other.entries }
}

impl RtList {
    pub fn new(entries: Vec<(RtValue, Option<RtValue>)>) -> Self {
        let mut list = Self::default();
        list.replace(entries);
        list
    }

    pub fn replace(&mut self, entries: Vec<(RtValue, Option<RtValue>)>) {
        self.entries = entries;
        self.reindex();
    }

    pub fn reindex(&mut self) {
        self.index.clear();
        for (i, (key, _)) in self.entries.iter().enumerate() {
            self.index.entry(key.hash_key()).or_default().push(i);
        }
    }

    pub fn push(&mut self, key: RtValue, value: Option<RtValue>) {
        self.index.entry(key.hash_key()).or_default().push(self.entries.len());
        self.entries.push((key, value));
    }

    pub fn position(&self, key: &RtValue) -> Option<usize> {
        self.index
            .get(&key.hash_key())?
            .iter()
            .copied()
            .find(|i| self.entries.get(*i).is_some_and(|(k, _)| k == key))
    }

    pub fn get(&self, key: &RtValue) -> RtValue {
        if let RtValue::Num(n) = key {
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

    pub fn contains(&self, value: &RtValue) -> bool { self.position(value).is_some() }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppearanceDelta {
    pub vars: Vec<(Identifier, core::types::Value)>,
    pub overlays: Vec<AppearanceDelta>,
    pub underlays: Vec<AppearanceDelta>,
}

impl From<i32> for RtValue {
    fn from(n: i32) -> Self { Self::Num(n as f32) }
}

impl From<usize> for RtValue {
    fn from(n: usize) -> Self { Self::Num(n as f32) }
}
