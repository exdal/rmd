use core::location::Location;
use std::rc::Rc;

use lexer::token::Token;
use rustc_hash::FxHashMap;

/// `#define M(x) foo##x`, `#define M(x) foo x`
#[derive(Clone, Debug)]
pub enum BodyPart<'a> {
    Token(Token<'a>),
    Space,
    /// `#PARAM`
    Stringify(&'a str),
    /// `##PARAM`
    Paste(&'a str),
}

#[derive(Clone, Debug)]
pub struct Parameter<'a> {
    pub name: &'a str,
    /// `#define LOG(args...)`
    pub variadic: bool,
}

/// `__LINE__`, `__FILE__`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Line,
    File,
}

/// `#define FOO`, `#define FOO()`
#[derive(Clone, Debug)]
pub struct Define<'a> {
    pub name: &'a str,
    pub params: Option<Vec<Parameter<'a>>>,
    pub body: Vec<BodyPart<'a>>,
    pub location: Location,
    pub builtin: Option<Builtin>,
}

impl<'a> Define<'a> {
    pub fn object_like(name: &'a str, body: Vec<BodyPart<'a>>, location: Location) -> Self {
        Self {
            name,
            params: None,
            body,
            location,
            builtin: None,
        }
    }

    pub fn builtin(name: &'static str, builtin: Builtin) -> Self {
        Self {
            name,
            params: None,
            body: Vec::new(),
            location: Location::default(),
            builtin: Some(builtin),
        }
    }

    pub fn is_function_like(&self) -> bool { self.params.is_some() }

    pub fn arity(&self) -> usize { self.params.as_ref().map_or(0, |p| p.len()) }

    pub fn lookup(&self, name: &str) -> Option<Slot> {
        let params = self.params.as_ref()?;

        if let Some(index) = params.iter().position(|p| !p.variadic && p.name == name) {
            return Some(Slot::Positional(index));
        }

        params.iter().position(|p| p.variadic && p.name == name).map(Slot::Rest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Positional(usize),
    /// `args...`
    Rest(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NameId(u32);

#[derive(Default)]
pub struct DefineTable<'a> {
    names: FxHashMap<&'a str, NameId>,
    defines: Vec<Option<Rc<Define<'a>>>>,
    len: usize,
}

impl<'a> DefineTable<'a> {
    pub fn new() -> Self { Self::default() }

    pub fn with_builtins() -> Self {
        let mut table = Self::default();
        table.define(Define::builtin("__LINE__", Builtin::Line));
        table.define(Define::builtin("__FILE__", Builtin::File));

        table
    }

    pub fn name_id(&mut self, name: &'a str) -> NameId {
        let next = NameId(self.defines.len() as u32);
        let id = *self.names.entry(name).or_insert(next);
        if id == next {
            self.defines.push(None);
        }

        id
    }

    pub fn find(&self, name: &str) -> Option<NameId> { self.names.get(name).copied() }

    pub fn define(&mut self, define: Define<'a>) -> Option<Rc<Define<'a>>> {
        let id = self.name_id(define.name);
        let previous = self.defines[id.0 as usize].replace(Rc::new(define));
        if previous.is_none() {
            self.len += 1;
        }

        previous
    }

    pub fn undef(&mut self, name: &str) -> Option<Rc<Define<'a>>> {
        let id = self.find(name)?;
        let previous = self.defines[id.0 as usize].take();
        if previous.is_some() {
            self.len -= 1;
        }

        previous
    }

    pub fn get(&self, name: &str) -> Option<&Define<'a>> { self.lookup_id(self.find(name)?).map(Rc::as_ref) }

    pub fn lookup(&self, name: &str) -> Option<(NameId, Rc<Define<'a>>)> {
        let id = self.find(name)?;

        self.lookup_id(id).map(|define| (id, define.clone()))
    }

    fn lookup_id(&self, id: NameId) -> Option<&Rc<Define<'a>>> { self.defines[id.0 as usize].as_ref() }

    pub fn is_defined(&self, name: &str) -> bool { self.get(name).is_some() }

    pub fn len(&self) -> usize { self.len }

    pub fn is_empty(&self) -> bool { self.len == 0 }
}
