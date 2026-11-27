pub mod define;
pub mod diagnostic;
mod directive;
pub mod error;
pub mod eval;
mod expand;
mod hide_set;
mod include;
mod output;
mod pragma;
mod prelude;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use base::{
    arena::StrArena,
    location::{FileId, Location},
    source::SourceMap,
};
use lexer::{IndentState, Lexer, token::Token};

use crate::{
    define::{BodyPart, DefineTable},
    diagnostic::{Code, Level, PragmaTable},
    error::PreprocessError,
    expand::Arguments,
    hide_set::{HideSet, HideSets},
    include::normalize,
    output::IndentOwner,
};

pub use self::prelude::{
    CORE_SOURCE, DEMIR_ENV, DEMIR_SOURCE, IMGUI_SOURCE, PreludeFile, STDDEF_ENV, STDDEF_EXT_SOURCE, STDDEF_SOURCE,
    VERSION_SOURCE, core_files, prelude_files,
};

pub type PreprocessResult<T> = Result<T, PreprocessError>;
pub type Spanned<'a> = (Token<'a>, Location);

/// Source text shared by multiple preprocessing configurations in one compilation.
#[derive(Clone, Default)]
pub struct SourceCache<'a> {
    files: HashMap<PathBuf, &'a str>,
}

impl SourceCache<'_> {
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Clone, Copy)]
enum Spacing {
    /// Tokens read directly from a source file can derive spacing from their locations.
    Source,
    Joint,
    Separated,
}

#[derive(Clone, Copy)]
struct RawToken<'a> {
    token: Token<'a>,
    location: Location,
    /// Macro expansion reuses the invocation location for every token, so its whitespace has to
    /// travel separately from the source span.
    spacing: Spacing,
    hidden: HideSet,
}

impl<'a> RawToken<'a> {
    fn source((token, location): Spanned<'a>) -> Self {
        Self {
            token,
            location,
            spacing: Spacing::Source,
            hidden: HideSet::default(),
        }
    }

    fn expanded(token: Token<'a>, location: Location, separated: bool, hidden: HideSet) -> Self {
        Self {
            token,
            location,
            spacing: if separated { Spacing::Separated } else { Spacing::Joint },
            hidden,
        }
    }

    fn separated_from(&self, previous: Location) -> bool {
        match self.spacing {
            Spacing::Source => !adjacent(previous, self.location),
            Spacing::Joint => false,
            Spacing::Separated => true,
        }
    }
}

/// `foo##x`, `foo x`
fn adjacent(left: Location, right: Location) -> bool {
    left.file == right.file && left.end.line == right.begin.line && left.end.col == right.begin.col
}

/// `foo##123`
fn pastable<'a>(token: &Token<'a>) -> Option<&'a str> {
    match token {
        Token::IntegerLiteral(text) | Token::FloatingPointLiteral(text) | Token::HexLiteral(text) => Some(text),
        other => other.word(),
    }
}

struct IncludeState<'a> {
    lexer: Lexer<'a>,
    /// `#include "dir/file.dm"`
    dir: PathBuf,
    peeked: Option<Spanned<'a>>,
}

pub struct Preprocessed<'a> {
    pub tokens: Vec<Spanned<'a>>,
    pub sources: SourceMap<'a>,
    pub entry: Option<FileId>,
    /// `<stddef.dm>`
    pub builtin_files: Vec<FileId>,
    /// `#include "map.dmm"`
    pub resources: Vec<PathBuf>,
    /// `#define FILE_DIR "icons"`
    pub resource_dirs: Vec<PathBuf>,
    pub defines: DefineTable<'a>,
    pub errors: Vec<PreprocessError>,
    pub source_cache: SourceCache<'a>,
}

impl Preprocessed<'_> {
    pub fn is_ok(&self) -> bool {
        !self.errors.iter().any(PreprocessError::is_fatal)
    }
}

type ProgressHook<'a> = Box<dyn FnMut(&Path) -> bool + 'a>;

pub struct Preprocessor<'a> {
    arena: &'a StrArena,
    sources: SourceMap<'a>,
    defines: DefineTable<'a>,
    include_stack: Vec<IncludeState<'a>>,
    // expanded tokens still to be read, last one first
    pending: Vec<RawToken<'a>>,
    // where each expansion starts in `pending`
    expansions: Vec<usize>,
    hide_sets: HideSets,
    arguments: Arguments<'a>,
    condition: Vec<Spanned<'a>>,
    stringified: String,
    /// `#if`, `#elif`, `#else`, `#endif`
    conditionals: Vec<Option<bool>>,
    pragmas: PragmaTable,
    output: Vec<Spanned<'a>>,
    builtin_files: Vec<FileId>,
    resources: Vec<PathBuf>,
    resource_dirs: Vec<PathBuf>,
    included: HashSet<PathBuf>,
    // files that are allowed to include more than once (pragma multiple)
    multiple: HashSet<PathBuf>,
    errors: Vec<PreprocessError>,

    /// Layout held until a line emits code
    carried_layout: Vec<Spanned<'a>>,
    line_layout: Vec<Spanned<'a>>,
    indent_owners: Vec<IndentOwner>,
    /// Indentation before the current line
    line_indent: Option<(usize, IndentState)>,
    line_has_content: bool,
    can_use_directive: bool,
    /// Last token read from a file
    last_file_line: Option<(FileId, u32)>,
    last_if: Location,

    include_core: bool,
    prelude: Vec<PreludeFile>,
    postlude: Vec<PreludeFile>,
    progress: Option<ProgressHook<'a>>,
    aborted: bool,
    source_cache: SourceCache<'a>,
}

impl<'a> Preprocessor<'a> {
    pub fn new(arena: &'a StrArena) -> Self {
        Self {
            arena,
            sources: SourceMap::new(),
            defines: DefineTable::with_builtins(),
            include_stack: Vec::new(),
            pending: Vec::new(),
            expansions: Vec::new(),
            hide_sets: HideSets::default(),
            arguments: Arguments::default(),
            condition: Vec::new(),
            stringified: String::new(),
            conditionals: Vec::new(),
            pragmas: PragmaTable::new(),
            output: Vec::new(),
            builtin_files: Vec::new(),
            resources: Vec::new(),
            resource_dirs: Vec::new(),
            included: HashSet::new(),
            multiple: HashSet::new(),
            errors: Vec::new(),
            carried_layout: Vec::new(),
            line_layout: Vec::new(),
            indent_owners: Vec::new(),
            line_indent: None,
            line_has_content: false,
            can_use_directive: true,
            last_file_line: None,
            last_if: Location::default(),
            include_core: true,
            prelude: prelude_files(),
            postlude: Vec::new(),
            progress: None,
            aborted: false,
            source_cache: SourceCache::default(),
        }
    }

    pub fn with_source_cache(mut self, cache: SourceCache<'a>) -> Self {
        self.source_cache = cache;

        self
    }

    /// reuse earlier pass's token buffer, so a second configuration skips growing it again
    pub fn with_output(mut self, mut output: Vec<Spanned<'a>>) -> Self {
        output.clear();
        self.output = output;

        self
    }

    pub fn with_prelude(mut self, files: impl IntoIterator<Item = PreludeFile>) -> Self {
        self.prelude = files.into_iter().collect();

        self
    }

    pub fn with_postlude(mut self, files: impl IntoIterator<Item = PreludeFile>) -> Self {
        self.postlude = files.into_iter().collect();

        self
    }

    pub fn with_baking(mut self, enabled: bool) -> Self {
        if enabled {
            self.prelude
                .insert(0, PreludeFile::Embedded("<demir-bake.dm>", "#define __DEMIR_BAKE__\n"));
        }

        self
    }

    pub fn with_progress(mut self, progress: impl FnMut(&Path) -> bool + 'a) -> Self {
        self.progress = Some(Box::new(progress));

        self
    }

    pub fn without_prelude(mut self) -> Self {
        self.prelude.clear();

        self
    }

    #[cfg(test)]
    fn without_core(mut self) -> Self {
        self.include_core = false;

        self
    }

    pub fn sources(&self) -> &SourceMap<'a> {
        &self.sources
    }

    pub fn run(mut self, entry: impl AsRef<Path>) -> PreprocessResult<Preprocessed<'a>> {
        let mut files = std::mem::take(&mut self.prelude);
        if self.include_core {
            files.splice(0..0, core_files());
        }
        for file in files {
            match file {
                PreludeFile::Embedded(name, contents) => self.open_embedded(name, contents),
                PreludeFile::Disk(path) => self.include_file(&path, Location::default()),
            }

            self.drain();
            self.end_stream();
        }

        let entry = normalize(entry.as_ref());
        let entry_errors = self.errors.len();
        self.include_file(&entry, Location::default());

        if self.include_stack.is_empty()
            && let Some(error) = self
                .errors
                .get(entry_errors..)
                .unwrap_or_default()
                .iter()
                .find(|e| e.is_fatal())
        {
            return Err(error.clone());
        }

        self.drain();
        self.end_stream();

        for file in std::mem::take(&mut self.postlude) {
            match file {
                PreludeFile::Embedded(name, contents) => self.open_embedded(name, contents),
                PreludeFile::Disk(path) => self.include_file(&path, Location::default()),
            }

            self.drain();
            self.end_stream();
        }

        if !self.conditionals.is_empty() {
            let count = self.conditionals.len();
            let plural = if count == 1 { "" } else { "s" };
            let location = self.last_if;
            self.diagnose(
                Code::BadDirective,
                location,
                format!("missing {count} #endif directive{plural}"),
            );
        }

        Ok(Preprocessed {
            entry: self.sources.find(&entry),
            tokens: self.output,
            sources: self.sources,
            builtin_files: self.builtin_files,
            resources: self.resources,
            resource_dirs: self.resource_dirs,
            defines: self.defines,
            errors: self.errors,
            source_cache: self.source_cache,
        })
    }

    fn diagnose(&mut self, code: Code, location: Location, message: impl Into<String>) {
        let level = self.pragmas.level(code);
        if level == Level::Disabled {
            return;
        }

        self.errors.push(PreprocessError::new(code, level, location, message));
    }
}

pub fn preprocess(arena: &StrArena, entry: impl AsRef<Path>) -> PreprocessResult<Preprocessed<'_>> {
    Preprocessor::new(arena).run(entry)
}

/// [`preprocess`], reporting each opened file. Returning `false` from `progress` aborts the run.
pub fn preprocess_with_progress<'a>(
    arena: &'a StrArena, entry: impl AsRef<Path>, progress: impl FnMut(&Path) -> bool + 'a,
) -> PreprocessResult<Preprocessed<'a>> {
    Preprocessor::new(arena).with_progress(progress).run(entry)
}

pub fn render(tokens: &[Spanned<'_>]) -> String {
    let mut out = String::new();

    for (token, _) in tokens {
        match token {
            Token::Newline => out.push('\n'),
            Token::SuppressedNewline => {},
            Token::Indent => out.push_str("> "),
            Token::Dedent => out.push_str("< "),
            token => {
                out.push_str(&token.to_string());
                out.push(' ');
            },
        }
    }

    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
}

pub fn macros_used_before_definition<'n>(
    defines: &DefineTable<'_>, sources: &SourceMap<'_>, root: &Path,
    uses: impl IntoIterator<Item = (&'n str, Location)>,
) -> Vec<PreprocessError> {
    let mut reported = HashSet::new();
    let mut errors = Vec::new();
    for (name, location) in uses {
        let Some(define) = defines.get(name) else {
            continue;
        };

        let is_self_reference =
            matches!(define.body.as_slice(), [BodyPart::Token(Token::Identifier(body))] if *body == name);
        if define.builtin.is_some() || define.is_function_like() || is_self_reference || !reported.insert(name) {
            continue;
        }

        let path = sources
            .path(define.location.file)
            .map(|path| path.strip_prefix(root).unwrap_or(path));
        errors.push(PreprocessError::new(
            Code::ItemDoesntExist,
            Code::ItemDoesntExist.default_level(),
            location,
            format!(
                "'{name}' is used before its #define at {}, macros only apply after they are defined",
                define.location.display(path)
            ),
        ));
    }

    errors
}

#[cfg(test)]
mod tests;
