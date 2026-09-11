use std::cell::RefCell;

/// An append-only arena of strings handing out `&'arena str`.
///
/// The preprocessor discovers `#include`s while it is already handing out tokens that borrow from
/// files it loaded earlier, so a `&mut SourceMap`-shaped API would deadlock the borrow checker and
/// force every token to own its text. Interior mutability plus boxed (address stable) strings lets
/// the whole pipeline stay zero copy: `Token<'a>` borrows straight out of the file buffer.
#[derive(Default)]
pub struct StrArena {
    chunks: RefCell<Vec<Box<str>>>,
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

    pub fn alloc_str(&self, s: &str) -> &str { self.alloc(s.to_string()) }

    pub fn len(&self) -> usize { self.chunks.borrow().len() }

    pub fn is_empty(&self) -> bool { self.chunks.borrow().is_empty() }
}
