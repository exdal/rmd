use core::{location::Location, types::Identifier};
use std::{collections::HashMap, rc::Rc};

use lexer::token::Token;

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
pub struct Parameter {
    pub name: Identifier,
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
    pub name: Identifier,
    pub params: Option<Vec<Parameter>>,
    pub body: Vec<BodyPart<'a>>,
    pub location: Location,
    pub builtin: Option<Builtin>,
}

impl<'a> Define<'a> {
    pub fn object_like(name: Identifier, body: Vec<BodyPart<'a>>, location: Location) -> Self {
        Self {
            name,
            params: None,
            body,
            location,
            builtin: None,
        }
    }

    pub fn builtin(name: &str, builtin: Builtin) -> Self {
        Self {
            name: Identifier::from(name),
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

        if let Some(index) = params.iter().position(|p| !p.variadic && p.name.as_str() == name) {
            return Some(Slot::Positional(index));
        }

        params
            .iter()
            .position(|p| p.variadic && p.name.as_str() == name)
            .map(Slot::Rest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Positional(usize),
    /// `args...`
    Rest(usize),
}

#[derive(Default)]
pub struct DefineTable<'a> {
    defines: HashMap<Identifier, Rc<Define<'a>>>,
}

impl<'a> DefineTable<'a> {
    pub fn new() -> Self { Self::default() }

    pub fn with_builtins() -> Self {
        let mut table = Self::default();
        table.define(Define::builtin("__LINE__", Builtin::Line));
        table.define(Define::builtin("__FILE__", Builtin::File));
        table.define(Define::object_like(
            Identifier::from("DM_VERSION"),
            vec![BodyPart::Token(Token::IntegerLiteral("515"))],
            Location::default(),
        ));
        table.define(Define::object_like(
            Identifier::from("DM_BUILD"),
            vec![BodyPart::Token(Token::IntegerLiteral("1642"))],
            Location::default(),
        ));

        table
    }

    pub fn define(&mut self, define: Define<'a>) -> Option<Rc<Define<'a>>> {
        self.defines.insert(define.name.clone(), Rc::new(define))
    }

    pub fn undef(&mut self, name: &Identifier) -> Option<Rc<Define<'a>>> { self.defines.remove(name) }

    pub fn get(&self, name: &str) -> Option<&Define<'a>> { self.defines.get(&Identifier::from(name)).map(Rc::as_ref) }

    pub fn lookup(&self, name: &str) -> Option<Rc<Define<'a>>> { self.defines.get(&Identifier::from(name)).cloned() }

    pub fn is_defined(&self, name: &str) -> bool { self.defines.contains_key(&Identifier::from(name)) }

    pub fn len(&self) -> usize { self.defines.len() }

    pub fn is_empty(&self) -> bool { self.defines.is_empty() }
}
