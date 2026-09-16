use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Weak},
};

#[derive(Debug)]
struct Interned {
    text: String,
    hash: u64,
}

#[derive(Clone)]
pub struct Symbol(Arc<Interned>);

impl Symbol {
    pub fn as_str(&self) -> &str { &self.0.text }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0.hash == other.0.hash && self.as_str() == other.as_str())
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) { state.write_u64(self.0.hash); }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.as_str().cmp(other.as_str()) }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.as_str().fmt(f) }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SymbolHasher(u64);

impl Hasher for SymbolHasher {
    fn finish(&self) -> u64 { self.0 }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 ^= self.0 >> 29;
    }
}

pub type SymbolMap<V> = HashMap<Symbol, V, std::hash::BuildHasherDefault<SymbolHasher>>;

thread_local! { static IDENTIFIERS: RefCell<Interner> = RefCell::new(Interner::new()); }

impl From<&str> for Symbol {
    fn from(s: &str) -> Self { IDENTIFIERS.with(|pool| pool.borrow_mut().intern(s)) }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self { Self::from(s.as_str()) }
}

#[derive(Debug, Default)]
pub struct Interner {
    buckets: HashMap<u64, Vec<Weak<Interned>>>,
    allocations: usize,
}

impl Interner {
    pub fn new() -> Self { Self::default() }

    fn hash(s: &str) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hash);

        hash.finish()
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(symbol) = self.get(s) {
            return symbol;
        }

        self.allocations = self.allocations.wrapping_add(1);

        if self.allocations.is_multiple_of(1024) {
            self.buckets.retain(|_, bucket| {
                bucket.retain(|s| s.strong_count() > 0);

                !bucket.is_empty()
            });
        }

        let hash = Self::hash(s);
        let value = Arc::new(Interned {
            text: s.to_string(),
            hash,
        });
        self.buckets.entry(hash).or_default().push(Arc::downgrade(&value));

        Symbol(value)
    }

    pub fn get(&self, s: &str) -> Option<Symbol> {
        self.buckets
            .get(&Self::hash(s))?
            .iter()
            .filter_map(Weak::upgrade)
            .find(|v| v.text == s)
            .map(Symbol)
    }

    pub fn len(&self) -> usize { self.buckets.values().flatten().filter(|s| s.strong_count() > 0).count() }

    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_survive_pools_and_compare_across_threads() {
        let mut pool = Interner::new();
        let a = pool.intern("icon_state");

        assert!(Arc::ptr_eq(&a.0, &pool.intern("icon_state").0));

        drop(pool);
        let b = std::thread::spawn(|| Symbol::from("icon_state")).join().unwrap();

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "icon_state");
    }
}
