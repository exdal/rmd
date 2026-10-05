use std::cell::RefCell;

const BUMP_CHUNK: usize = 64 * 1024;

#[derive(Default)]
pub struct StrArena {
    chunks: RefCell<Vec<Box<str>>>,
    bump: RefCell<Bump>,
}

#[derive(Default)]
struct Bump {
    full: Vec<Box<[u8]>>,
    current: Box<[u8]>,
    used: usize,
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
        if bump.current.len() - bump.used < len {
            let full = std::mem::replace(&mut bump.current, vec![0; BUMP_CHUNK].into_boxed_slice());
            bump.full.push(full);
            bump.used = 0;
        }

        let start = bump.used;
        for part in parts {
            let end = bump.used + part.len();
            bump.current[bump.used..end].copy_from_slice(part.as_bytes());
            bump.used = end;
        }

        let ptr = bump.current[start..start + len].as_ptr();

        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
    }

    pub fn len(&self) -> usize { self.chunks.borrow().len() }

    pub fn is_empty(&self) -> bool { self.chunks.borrow().is_empty() }
}
