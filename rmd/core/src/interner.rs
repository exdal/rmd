use std::collections::HashMap;

/// An interned string. A full SS13 tree is on the order of a hundred thousand path segments, most of
/// them repeats of `obj`, `item`, `structure`, ... so the tree stores symbols instead of strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(pub u32);

#[derive(Debug, Default)]
pub struct Interner {
    map: HashMap<String, Symbol>,
    strings: Vec<String>,
}

impl Interner {
    pub fn new() -> Self { Self::default() }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(symbol) = self.map.get(s) {
            return *symbol;
        }

        let symbol = Symbol(self.strings.len() as u32);
        self.strings.push(s.to_string());
        self.map.insert(s.to_string(), symbol);

        symbol
    }

    pub fn get(&self, s: &str) -> Option<Symbol> { self.map.get(s).copied() }

    pub fn resolve(&self, symbol: Symbol) -> Option<&str> { self.strings.get(symbol.0 as usize).map(|s| s.as_str()) }

    pub fn len(&self) -> usize { self.strings.len() }

    pub fn is_empty(&self) -> bool { self.strings.is_empty() }
}
