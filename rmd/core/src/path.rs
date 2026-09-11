use crate::types::Identifier;

bitflags::bitflags! {
    /// ```text
    /// /datum/foo            -> IS_DATUM
    /// /datum/foo/proc/bar   -> IS_DATUM | IS_PROC
    /// /datum/foo/var/baz    -> IS_DATUM | IS_VAR
    /// /var/global/gamemode  -> IS_VAR   | IS_GLOBAL
    /// ```
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PathFlags: u32 {
        const IS_PROC = 1 << 0;
        const IS_DATUM = 1 << 1;
        const IS_GLOBAL = 1 << 2;
        const IS_VAR = 1 << 3;
        const IS_VERB = 1 << 4;
        const IS_CONST = 1 << 5;
        const IS_TMP = 1 << 6;
        const IS_STATIC = 1 << 7;
        const IS_OPERATOR = 1 << 8;
        const IS_FINAL = 1 << 9;
        const KEYWORD = Self::IS_PROC.bits() | Self::IS_VERB.bits() | Self::IS_VAR.bits();
        const NONE = 0;
    }
}

/// The operator that joined two path segments together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOp {
    /// `/` - normal tree descent
    Slash,
    /// `.` - proc-local scope
    Dot,
    /// `:` - unchecked runtime scope
    Colon,
}

/// A path in the object tree, e.g. `/obj/item/weapon/sword`.
///
/// Reserved segments (`proc`, `verb`, `var`, `global`, `static`, `const`, `tmp`) are stripped out
/// into [`PathFlags`] so that `segments` only ever holds real path segments. Two offsets say how to
/// read what is left:
///
/// ```text
/// /obj/foo/var/mob/living/target
///  ^^^^^^^     ^^^^^^^^^^ ^^^^^^
///  owner       declared   name
///              type
/// ```
///
/// `keyword_offset` is where the `var`/`proc`/`verb` keyword sat, which is the only thing that tells
/// `/obj/foo/var/bar` (a var `bar` on `/obj/foo`) apart from `var/obj/foo/bar` (a var `bar` typed
/// `/obj/foo`).
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreePath {
    pub segments: Vec<Identifier>,
    pub absolute: bool,
    pub flags: PathFlags,
    pub name_offset: usize,
    pub keyword_offset: usize,
}

impl TreePath {
    pub fn new(segments: Vec<Identifier>, absolute: bool) -> Self {
        let name_offset = segments.len().saturating_sub(1);
        Self {
            keyword_offset: segments.len(),
            segments,
            absolute,
            flags: PathFlags::NONE,
            name_offset,
        }
    }

    /// Split a textual path (`/obj/item/proc/attack`) into segments and flags.
    pub fn parse(s: &str) -> Self {
        let absolute = s.starts_with('/');
        let mut flags = PathFlags::NONE;
        let mut segments = Vec::new();
        let mut keyword_offset = None;

        for part in s.split('/').filter(|p| !p.is_empty()) {
            match part {
                "proc" | "verb" | "var" => {
                    flags |= match part {
                        "proc" => PathFlags::IS_PROC,
                        "verb" => PathFlags::IS_VERB,
                        _ => PathFlags::IS_VAR,
                    };

                    keyword_offset.get_or_insert(segments.len());
                },
                "global" => flags |= PathFlags::IS_GLOBAL,
                "static" => flags |= PathFlags::IS_STATIC,
                "const" => flags |= PathFlags::IS_CONST,
                "final" => flags |= PathFlags::IS_FINAL,
                "tmp" => flags |= PathFlags::IS_TMP,
                _ => segments.push(Identifier(part.to_string())),
            }
        }

        if !flags.contains(PathFlags::IS_PROC)
            && !flags.contains(PathFlags::IS_VERB)
            && !flags.contains(PathFlags::IS_VAR)
        {
            flags |= PathFlags::IS_DATUM;
        }

        let name_offset = segments.len().saturating_sub(1);
        Self {
            keyword_offset: keyword_offset.unwrap_or(segments.len()),
            segments,
            absolute,
            flags,
            name_offset,
        }
    }

    /// The final segment - the name of the datum, proc or var being declared.
    pub fn name(&self) -> Option<&Identifier> { self.segments.get(self.name_offset) }

    /// The type that owns this declaration: everything before the `var`/`proc` keyword.
    pub fn owner(&self) -> &[Identifier] { &self.segments[..self.keyword_offset.min(self.segments.len())] }

    /// The type a declaration attaches to, whether or not a `var`/`proc` keyword was spelled out.
    /// `/obj/foo/proc/bar` and `/obj/foo/bar` (a proc override) both hang off `/obj/foo`.
    pub fn declaration_owner(&self) -> &[Identifier] {
        if self.keyword_offset >= self.segments.len() {
            &self.segments[..self.name_offset]
        } else {
            self.owner()
        }
    }

    /// The declared type of a var, between the keyword and the name.
    pub fn declared_type(&self) -> &[Identifier] {
        let begin = self.keyword_offset.min(self.segments.len());
        let end = self.name_offset.max(begin).min(self.segments.len());

        &self.segments[begin..end]
    }

    pub fn is_root(&self) -> bool { self.segments.is_empty() }

    pub fn len(&self) -> usize { self.segments.len() }

    pub fn is_empty(&self) -> bool { self.segments.is_empty() }

    /// `/obj/item` + `sword` -> `/obj/item/sword`
    pub fn join(&self, segment: Identifier) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment);

        Self::new(segments, self.absolute)
    }

    /// Concatenate a relative path onto this one. Used when a child block is nested under a parent
    /// block by indentation.
    pub fn concat(&self, other: &Self) -> Self {
        let prefix_len = self.segments.len();
        let mut segments = self.segments.clone();
        segments.extend(other.segments.iter().cloned());

        // The keyword belongs to whichever half actually spelled it out. `keyword_offset` alone
        // cannot answer that: a prefix which is nothing but the keyword (`var`, `mob/proc`) parks
        // it at `segments.len()`, exactly where an absent keyword also sits.
        let keyword_offset = if self.flags.intersects(PathFlags::KEYWORD) {
            self.keyword_offset
        } else {
            prefix_len + other.keyword_offset
        };

        Self {
            name_offset: segments.len().saturating_sub(1),
            keyword_offset: keyword_offset.min(segments.len()),
            segments,
            absolute: self.absolute,
            flags: self.flags | other.flags,
        }
    }

    /// The next entry in a comma-separated var list, which shares everything up to the name:
    /// `var/mob/M` + `N` -> `var/mob/N`.
    pub fn sibling(&self, other: &Self) -> Self {
        let mut segments = self.segments[..self.name_offset.min(self.segments.len())].to_vec();
        segments.extend(other.segments.iter().cloned());

        Self {
            name_offset: segments.len().saturating_sub(1),
            keyword_offset: self.keyword_offset.min(segments.len()),
            segments,
            absolute: self.absolute,
            flags: self.flags | other.flags,
        }
    }

    /// Whether `self` is `other` or lives underneath it.
    pub fn starts_with(&self, other: &Self) -> bool { self.segments.starts_with(&other.segments) }
}

impl std::fmt::Display for TreePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.segments.is_empty() {
            return write!(f, "/");
        }

        for segment in &self.segments {
            write!(f, "/{segment}")?;
        }

        Ok(())
    }
}
