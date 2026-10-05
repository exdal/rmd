use std::cell::RefCell;

const BUMP_CHUNK: usize = 64 * 1024;

#[derive(Default)]
pub struct StrArena {
    chunks: RefCell<Vec<Box<str>>>,
    bump: RefCell<Bump>,
}

#[derive(Default)]
struct Bump {
    full: Vec<Vec<u8>>,
    current: Vec<u8>,
}

impl StrArena {
    pub fn new() -> Self { Self::default() }

    pub fn alloc(&self, s: String) -> &str {
        let boxed = s.into_boxed_str();
        let ptr: *const str = &*boxed;
        self.chunks.borrow_mut().push(boxed);

        // SAFETY: the `str` lives on the heap behind a `Box` that is never mutated, moved out of, or
        // dropped before the arena itself. Growing `chunks` moves the `Box`, never the `str`.
        unsafe { &*ptr }
    }

    pub fn alloc_str(&self, s: &str) -> &str { self.alloc_concat(&[s]) }

    pub fn alloc_concat(&self, parts: &[&str]) -> &str {
        let len = parts.iter().map(|part| part.len()).sum::<usize>();
        if len > BUMP_CHUNK / 4 {
            return self.alloc(parts.concat());
        }

        let bump = &mut *self.bump.borrow_mut();
        if bump.current.capacity() - bump.current.len() < len {
            let full = std::mem::replace(&mut bump.current, Vec::with_capacity(BUMP_CHUNK));
            bump.full.push(full);
        }

        let start = bump.current.len();
        for part in parts {
            bump.current.extend_from_slice(part.as_bytes());
        }

        unsafe {
            let bytes = std::slice::from_raw_parts(bump.current.as_ptr().add(start), len);
            std::str::from_utf8_unchecked(bytes)
        }
    }

    pub fn len(&self) -> usize { self.chunks.borrow().len() }

    pub fn is_empty(&self) -> bool { self.chunks.borrow().is_empty() }
}
