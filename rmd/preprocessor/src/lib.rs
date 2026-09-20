pub mod define;
pub mod diagnostic;
pub mod error;
pub mod eval;

use core::{
    arena::StrArena,
    location::{FileId, Location},
    source::SourceMap,
    types::Identifier,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Component, Path, PathBuf},
    rc::Rc,
};

use lexer::{IndentState, Lexer, token::Token};

use crate::{
    define::{BodyPart, Builtin, Define, DefineTable, Parameter, Slot},
    diagnostic::{Code, Level, PragmaTable},
    error::PreprocessError,
};

pub type PreprocessResult<T> = Result<T, PreprocessError>;
pub type Spanned<'a> = (Token<'a>, Location);

/// Source text shared by multiple preprocessing configurations in one compilation.
#[derive(Clone, Default)]
pub struct SourceCache<'a> {
    files: HashMap<PathBuf, &'a str>,
}

impl SourceCache<'_> {
    pub fn len(&self) -> usize { self.files.len() }

    pub fn is_empty(&self) -> bool { self.files.is_empty() }
}

#[derive(Clone, Copy)]
enum Spacing {
    /// Tokens read directly from a source file can derive spacing from their locations.
    Source,
    Joint,
    Separated,
}

#[derive(Clone, Default)]
struct HideSet(Option<Rc<HiddenMacro>>);

struct HiddenMacro {
    name: Identifier,
    previous: HideSet,
}

impl HideSet {
    fn contains(&self, name: &str) -> bool {
        let mut current = self.0.as_deref();
        while let Some(hidden) = current {
            if hidden.name.as_str() == name {
                return true;
            }

            current = hidden.previous.0.as_deref();
        }

        false
    }

    fn with(&self, name: &Identifier) -> Self {
        if self.contains(name.as_str()) {
            return self.clone();
        }

        Self(Some(Rc::new(HiddenMacro {
            name: name.clone(),
            previous: self.clone(),
        })))
    }

    fn union(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        let mut current = other.0.as_deref();
        while let Some(hidden) = current {
            merged = merged.with(&hidden.name);
            current = hidden.previous.0.as_deref();
        }

        merged
    }
}

#[derive(Clone)]
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

struct Expansion<'a> {
    tokens: VecDeque<RawToken<'a>>,
}

pub struct Preprocessed<'a> {
    pub tokens: Vec<Spanned<'a>>,
    pub sources: SourceMap<'a>,
    pub entry: Option<FileId>,
    /// `#include "map.dmm"`
    pub resources: Vec<PathBuf>,
    /// `#define FILE_DIR "icons"`
    pub resource_dirs: Vec<PathBuf>,
    pub defines: DefineTable<'a>,
    pub errors: Vec<PreprocessError>,
    pub source_cache: SourceCache<'a>,
}

impl Preprocessed<'_> {
    pub fn is_ok(&self) -> bool { !self.errors.iter().any(PreprocessError::is_fatal) }
}

type ProgressHook<'a> = Box<dyn FnMut(&Path) -> bool + 'a>;

pub struct Preprocessor<'a> {
    arena: &'a StrArena,
    sources: SourceMap<'a>,
    defines: DefineTable<'a>,
    include_stack: Vec<IncludeState<'a>>,
    expansions: Vec<Expansion<'a>>,
    /// `#if`, `#elif`, `#else`, `#endif`
    conditionals: Vec<Option<bool>>,
    pragmas: PragmaTable,
    output: Vec<Spanned<'a>>,
    resources: Vec<PathBuf>,
    resource_dirs: Vec<PathBuf>,
    included: HashSet<PathBuf>,
    errors: Vec<PreprocessError>,

    /// Layout held until a line emits code
    carried_layout: Vec<Spanned<'a>>,
    line_layout: Vec<Spanned<'a>>,
    /// Indentation before the current line
    line_indent: Option<(usize, IndentState)>,
    line_has_content: bool,
    can_use_directive: bool,
    /// Last token read from a file
    last_file_line: Option<(FileId, usize)>,
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
            expansions: Vec::new(),
            conditionals: Vec::new(),
            pragmas: PragmaTable::new(),
            output: Vec::new(),
            resources: Vec::new(),
            resource_dirs: Vec::new(),
            included: HashSet::new(),
            errors: Vec::new(),
            carried_layout: Vec::new(),
            line_layout: Vec::new(),
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

    pub fn sources(&self) -> &SourceMap<'a> { &self.sources }

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

    fn drain(&mut self) {
        while !self.include_stack.is_empty() {
            if self.aborted {
                self.include_stack.clear();

                break;
            }

            self.pump();
        }
    }

    fn end_stream(&mut self) {
        self.flush_layout();

        if self.line_has_content {
            self.line_has_content = false;
            let location = self.output.last().map(|(_, location)| *location).unwrap_or_default();
            self.output.push((Token::Newline, location));
        }

        self.can_use_directive = true;
        self.line_indent = None;
    }

    fn pump(&mut self) {
        let from_file = self.expansions.is_empty();
        let Some(mut raw) = self.next_raw() else {
            return;
        };
        let token = raw.token;
        let location = raw.location;

        if from_file {
            let previous = self.last_file_line.replace((location.file, location.end.line));
            if previous != Some((location.file, location.begin.line)) {
                self.can_use_directive = true;
            }
        }

        match token {
            Token::SuppressedNewline => self.can_use_directive = true,
            Token::Newline => {
                self.can_use_directive = true;

                if self.line_has_content {
                    self.line_has_content = false;
                    self.output.push((token, location));
                }
            },
            Token::Indent | Token::Dedent => self.line_layout.push((token, location)),
            token if token.is_comment() => {},
            token if token.is_directive() => self.handle_directive(token, location, from_file),
            token => {
                let (token, location) = if from_file {
                    self.glue_identifier(token, location)
                } else {
                    (token, location)
                };
                raw.token = token;
                raw.location = location;

                if let Some(name) = token.word()
                    && self.try_macro(name, raw)
                {
                    return;
                }

                self.emit(token, location);
            },
        }
    }

    fn emit(&mut self, token: Token<'a>, location: Location) {
        let mut carried = std::mem::take(&mut self.carried_layout);
        self.output.append(&mut carried);
        let mut line = std::mem::take(&mut self.line_layout);
        self.output.append(&mut line);

        self.line_has_content = true;
        self.can_use_directive = token == Token::Semicolon;
        self.output.push((token, location));
    }

    fn flush_layout(&mut self) {
        let mut carried = std::mem::take(&mut self.carried_layout);
        self.output.append(&mut carried);
        let mut line = std::mem::take(&mut self.line_layout);
        self.output.append(&mut line);
    }

    fn next_raw(&mut self) -> Option<RawToken<'a>> {
        loop {
            while let Some(expansion) = self.expansions.last_mut() {
                if let Some(token) = expansion.tokens.pop_front() {
                    return Some(token);
                }

                self.expansions.pop();
            }

            let depth = self.include_stack.len();
            let state = self.include_stack.last()?;
            if state.peeked.is_none() && state.lexer.at_line_start() {
                let indent = state.lexer.indent_state();
                self.line_indent = Some((depth, indent));
                let mut line = std::mem::take(&mut self.line_layout);
                self.carried_layout.append(&mut line);
            }

            let state = self.include_stack.last_mut()?;
            let spanned = state.peeked.take().unwrap_or_else(|| state.lexer.advance());
            if spanned.0 != Token::Eof {
                return Some(RawToken::source(spanned));
            }

            let errors = state.lexer.take_errors();
            let finished = self.include_stack.pop()?;

            self.hand_back_indent(&finished);
            self.flush_layout();

            for error in errors {
                self.diagnose(Code::BadToken, error.location, error.kind.to_string());
            }
        }
    }

    /// Include indentation returns to its parent
    fn hand_back_indent(&mut self, finished: &IncludeState<'a>) {
        if let Some(parent) = self.include_stack.last_mut() {
            parent
                .lexer
                .restore_indent(finished.lexer.indent_state().without_errors());
            return;
        }

        let position = finished.lexer.position();
        let location = Location::in_file(finished.lexer.file(), position, position);
        for _ in 0..finished.lexer.open_indents() {
            self.line_layout.push((Token::Dedent, location));
        }
    }

    fn next_significant(&mut self) -> Option<RawToken<'a>> {
        loop {
            let raw = self.next_raw()?;
            if !raw.token.is_comment() {
                return Some(raw);
            }
        }
    }

    fn push_back(&mut self, raw: RawToken<'a>) {
        self.expansions.push(Expansion {
            tokens: VecDeque::from([raw]),
        });
    }

    fn check(&mut self, expected: Token<'a>) -> bool {
        let Some(spanned) = self.next_significant() else {
            return false;
        };

        if spanned.token == expected {
            return true;
        }

        self.push_back(spanned);

        false
    }

    fn skip_rest_of_line(&mut self) {
        while let Some(raw) = self.next_raw() {
            if matches!(raw.token, Token::Newline | Token::SuppressedNewline) {
                break;
            }
        }
    }

    /// `ES\KE`
    fn glue_identifier(&mut self, token: Token<'a>, location: Location) -> Spanned<'a> {
        if token.word().is_none() {
            return (token, location);
        }

        let mut text: Option<String> = None;
        let mut end = location;

        while let Some((next, next_location)) = self.peek_file() {
            let Some(spelling) = pastable(&next).filter(|_| adjacent(end, next_location)) else {
                break;
            };

            self.take_peeked();
            text.get_or_insert_with(|| token.word().unwrap_or_default().to_string())
                .push_str(spelling);
            end = next_location;
        }

        match text {
            Some(text) => (
                Token::from_identifier(self.arena.alloc(text)),
                Location::in_file(location.file, location.begin, end.end),
            ),
            None => (token, location),
        }
    }

    fn peek_file(&mut self) -> Option<Spanned<'a>> {
        let state = self.include_stack.last_mut()?;
        if state.peeked.is_none() {
            state.peeked = Some(state.lexer.advance());
        }

        state.peeked
    }

    fn take_peeked(&mut self) {
        if let Some(state) = self.include_stack.last_mut() {
            state.peeked = None;
        }
    }

    fn handle_directive(&mut self, directive: Token<'a>, location: Location, from_file: bool) {
        // `#define DEFINE #define`
        if from_file && !self.can_use_directive {
            self.diagnose(
                Code::MisplacedDirective,
                location,
                "there can only be whitespace before a preprocessor directive",
            );
            self.skip_rest_of_line();
            return;
        }

        // `\t#define X 1`
        if !self.line_has_content {
            self.rewind_directive_line();
        }

        match directive {
            Token::PpDefine => self.directive_define(location),
            Token::PpUndef => self.directive_undef(),
            Token::PpInclude => self.directive_include(location),
            Token::PpIf => self.directive_if(location),
            Token::PpIfdef => self.directive_ifdef(location, false),
            Token::PpIfndef => self.directive_ifdef(location, true),
            Token::PpElif => self.directive_elif(location),
            Token::PpElse => self.directive_else(location),
            Token::PpEndif => self.directive_endif(location),
            Token::PpPragma => self.directive_pragma(),
            Token::PpWarn(text) => self.diagnose(Code::WarningDirective, location, format!("#warn{text}")),
            Token::PpError(text) => self.diagnose(Code::ErrorDirective, location, format!("#error{text}")),
            other => self.diagnose(Code::BadDirective, location, format!("unexpected directive '{other}'")),
        }
    }

    /// `\t#define X 1`
    fn rewind_directive_line(&mut self) {
        self.line_layout.clear();

        let Some((depth, indent)) = self.line_indent.take() else {
            return;
        };

        if depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_indent(indent);
        }
    }

    fn directive_define(&mut self, location: Location) {
        let Some(name_raw) = self.next_significant() else {
            return;
        };
        let name_token = name_raw.token;
        let name_location = name_raw.location;

        let Some(name) = name_token.word() else {
            self.diagnose(
                Code::BadDirective,
                name_location,
                "unexpected token, identifier expected for #define directive",
            );
            if name_token != Token::Newline {
                self.skip_rest_of_line();
            }

            return;
        };

        if name == "FILE_DIR" {
            self.directive_file_dir();
            return;
        }

        if name == "defined" {
            self.diagnose(
                Code::SoftReservedKeyword,
                name_location,
                "reserved keyword 'defined' cannot be used as a macro name",
            );
        }

        // `#define M (x) y`
        let mut params = None;
        if let Some(raw) = self.next_raw() {
            if raw.token == Token::ParenLeft && !raw.separated_from(name_location) {
                params = Some(self.read_parameters(raw.location));
            } else {
                self.push_back(raw);
            }
        }

        let body = self.read_macro_body();
        self.defines.define(Define {
            name: Identifier::from(name),
            params,
            body,
            location,
            builtin: None,
        });
    }

    /// `#define FILE_DIR "icons"`
    fn directive_file_dir(&mut self) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let directory = match token {
            Token::StringLiteral(text) | Token::RawStringLiteral(text) => Some(text.replace('\\', "/")),
            Token::Dot => Some(".".to_string()),
            _ => None,
        };

        let Some(directory) = directory else {
            self.diagnose(
                Code::InvalidFileDirDefine,
                token_location,
                format!("'{token}' is not a valid directory"),
            );
            self.skip_rest_of_line();
            return;
        };

        let base = self.include_stack.last().map(|s| s.dir.clone()).unwrap_or_default();
        self.resource_dirs.push(normalize(&base.join(directory)));
        self.skip_rest_of_line();
    }

    fn read_parameters(&mut self, open: Location) -> Vec<Parameter> {
        let mut params: Vec<Parameter> = Vec::new();
        let mut can_consume_comma = false;
        let mut found_variadic = false;

        loop {
            let Some(raw) = self.next_significant() else {
                self.diagnose(Code::BadDirective, open, "missing ')' in macro definition");
                return params;
            };
            let token = raw.token;
            let location = raw.location;

            match token {
                Token::ParenRight => return params,
                Token::Newline | Token::SuppressedNewline => {
                    self.push_back(raw);
                    self.diagnose(Code::BadDirective, open, "missing ')' in macro definition");
                    return params;
                },
                Token::Comma => {
                    if !can_consume_comma {
                        self.diagnose(Code::BadDirective, location, "unexpected ',' in macro parameter list");
                    }

                    can_consume_comma = false;
                },
                // `#define M(...)`
                Token::Ellipsis => {
                    can_consume_comma = true;
                    if self.reject_late_variadic(&params, location, &mut found_variadic) {
                        continue;
                    }

                    params.push(Parameter {
                        name: Identifier::from("..."),
                        variadic: true,
                    });
                    found_variadic = true;
                },
                token => {
                    let Some(name) = token.word() else {
                        self.diagnose(Code::BadDirective, location, "expected a macro parameter");
                        return params;
                    };

                    can_consume_comma = true;
                    if self.reject_late_variadic(&params, location, &mut found_variadic) {
                        continue;
                    }

                    let variadic = self.check(Token::Ellipsis);
                    found_variadic |= variadic;
                    params.push(Parameter {
                        name: Identifier::from(name),
                        variadic,
                    });
                },
            }
        }
    }

    /// `#define M(rest..., value)`
    fn reject_late_variadic(&mut self, params: &[Parameter], location: Location, found: &mut bool) -> bool {
        if !*found {
            return false;
        }

        let name = params.last().map(|p| p.name.to_string()).unwrap_or_default();
        self.diagnose(
            Code::BadDirective,
            location,
            format!("variadic argument '{name}' must be the last argument"),
        );
        *found = false;

        true
    }

    fn read_macro_body(&mut self) -> Vec<BodyPart<'a>> {
        let mut body: Vec<BodyPart<'a>> = Vec::new();
        let mut previous: Option<Location> = None;

        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            if matches!(token, Token::Newline | Token::SuppressedNewline) {
                break;
            }

            if token.is_comment() {
                continue;
            }

            let separated = previous.is_some_and(|p| raw.separated_from(p));
            previous = Some(location);

            match token {
                // `#PARAM`, `##PARAM`
                Token::Hash | Token::Concat => {
                    let Some(next_raw) = self.next_raw() else {
                        break;
                    };
                    let next = next_raw.token;
                    let next_location = next_raw.location;

                    let Some(name) = pastable(&next) else {
                        self.push_back(next_raw);
                        continue;
                    };

                    if token == Token::Hash {
                        if separated {
                            body.push(BodyPart::Space);
                        }

                        body.push(BodyPart::Stringify(name));
                    } else {
                        body.push(BodyPart::Paste(name));
                    }

                    previous = Some(next_location);
                },
                token => {
                    if separated {
                        body.push(BodyPart::Space);
                    }

                    body.push(BodyPart::Token(token));
                },
            }
        }

        while matches!(body.last(), Some(BodyPart::Space)) {
            body.pop();
        }

        body
    }

    fn directive_undef(&mut self) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let Some(name) = token.word() else {
            self.diagnose(Code::BadDirective, token_location, "invalid macro identifier");
            return;
        };

        if self.defines.undef(&Identifier::from(name)).is_none() {
            self.diagnose(
                Code::UndefineMissingDirective,
                token_location,
                format!("no macro named \"{name}\""),
            );
        }
    }

    fn directive_include(&mut self, location: Location) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let (Token::StringLiteral(raw) | Token::RawStringLiteral(raw)) = token else {
            self.diagnose(
                Code::InvalidInclusion,
                token_location,
                format!("'{token}' is not a valid include path"),
            );
            return;
        };

        let path = self.resolve_include(raw);
        self.include_file(&path, location);
    }

    fn directive_if(&mut self, location: Location) {
        self.last_if = location;

        let tokens = self.read_condition();
        if tokens.is_empty() {
            self.diagnose(Code::BadDirective, location, "expression expected for #if");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        }

        let is_defined = |name: &str| self.defines.is_defined(name);
        let base = self.include_stack.last().map(|s| s.dir.clone()).unwrap_or_default();
        let file_exists = |path: &str| base.join(path.replace('\\', "/")).is_file();

        let (value, complaints) = eval::evaluate(
            &tokens,
            &eval::Context {
                is_defined: &is_defined,
                file_exists: &file_exists,
            },
        );

        for complaint in complaints {
            self.diagnose(Code::BadDirective, complaint.location, complaint.message);
        }

        let Some(value) = value else {
            self.diagnose(Code::BadDirective, location, "expression is invalid");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        };

        let taken = value != 0.0;
        self.conditionals.push(Some(taken));

        if !taken {
            self.skip_if_body(false);
        }
    }

    /// `#if defined(X) && fexists("x")`
    fn read_condition(&mut self) -> Vec<Spanned<'a>> {
        let mut tokens = Vec::new();
        let mut expand = true;

        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            if matches!(token, Token::Newline | Token::SuppressedNewline) {
                break;
            }

            if token.is_comment() {
                continue;
            }

            if let Some(word) = token.word() {
                if word == "defined" || word == "fexists" {
                    expand = false;
                } else if expand && self.try_macro(word, raw) {
                    continue;
                }
            } else if token == Token::ParenRight {
                expand = true;
            }

            tokens.push((token, location));
        }

        tokens
    }

    fn directive_ifdef(&mut self, location: Location, negated: bool) {
        self.last_if = location;

        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let Some(name) = token.word() else {
            self.diagnose(Code::BadDirective, token_location, "expected a define identifier");
            self.conditionals.push(Some(false));
            self.skip_if_body(false);
            return;
        };

        let taken = self.defines.is_defined(name) != negated;
        self.conditionals.push(Some(taken));

        if !taken {
            self.skip_if_body(false);
        }
    }

    fn directive_elif(&mut self, location: Location) {
        match self.conditionals.last().copied() {
            None => {
                self.diagnose(Code::BadDirective, location, "unexpected #elif");
                self.skip_if_body(false);
            },
            Some(None) => {
                self.diagnose(
                    Code::BadDirective,
                    location,
                    "directive #elif cannot appear after #else in its flow control",
                );
                self.skip_if_body(false);
            },
            Some(Some(true)) => self.skip_if_body(false),
            Some(Some(false)) => {
                self.conditionals.pop();
                self.directive_if(location);
            },
        }
    }

    fn directive_else(&mut self, location: Location) {
        match self.conditionals.pop() {
            Some(Some(true)) => self.skip_if_body(true),
            Some(Some(false)) => self.conditionals.push(None),
            Some(None) => {
                self.conditionals.push(None);
                self.diagnose(Code::BadDirective, location, "unexpected #else");
            },
            None => self.diagnose(Code::BadDirective, location, "unexpected #else"),
        }
    }

    fn directive_endif(&mut self, location: Location) {
        if self.conditionals.pop().is_none() {
            self.diagnose(Code::BadDirective, location, "unexpected #endif");
        }
    }

    fn directive_pragma(&mut self) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let code = match token {
            token if token.word().is_some() => Code::from_name(token.word().unwrap_or_default()),
            Token::IntegerLiteral(text) => text.parse::<u16>().ok().and_then(Code::from_number),
            _ => {
                self.diagnose(
                    Code::BadDirective,
                    token_location,
                    format!("invalid warning identifier '{token}'"),
                );
                self.skip_rest_of_line();
                return;
            },
        };

        let Some(code) = code else {
            self.diagnose(
                Code::InvalidWarningCode,
                token_location,
                format!("warning '{token}' does not exist"),
            );
            self.skip_rest_of_line();
            return;
        };

        let Some(level_raw) = self.next_significant() else {
            return;
        };
        let level_token = level_raw.token;
        let level_location = level_raw.location;

        let level = level_token
            .word()
            .map(str::to_ascii_lowercase)
            .and_then(|word| Level::from_pragma(&word));

        let Some(level) = level else {
            self.diagnose(
                Code::BadDirective,
                level_location,
                "warnings can only be set to disabled, notice, warning, or error",
            );
            return;
        };

        if !self.pragmas.set(code, level) {
            self.diagnose(
                Code::BadDirective,
                token_location,
                format!("{code} cannot be set - it must always be an error"),
            );
        }
    }

    /// `#if 0 ... #endif`
    fn skip_if_body(&mut self, from_else: bool) {
        let carried = std::mem::take(&mut self.carried_layout);
        let line = std::mem::take(&mut self.line_layout);
        let line_indent = self.line_indent.take();
        let indent = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.indent_state()));
        let nesting = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.nesting_state()));
        let errors = self
            .include_stack
            .last()
            .map(|state| (self.include_stack.len(), state.lexer.error_count()));

        let mut depth = 1usize;
        let mut closed = false;
        while let Some(raw) = self.next_raw() {
            let token = raw.token;
            let location = raw.location;
            match token {
                Token::PpIf | Token::PpIfdef | Token::PpIfndef => depth += 1,
                Token::PpEndif => {
                    depth -= 1;
                    if depth == 0 {
                        if !from_else {
                            self.push_back(raw);
                        }

                        closed = true;
                        break;
                    }
                },
                Token::PpElse | Token::PpElif if depth == 1 => {
                    if from_else {
                        self.diagnose(Code::BadDirective, location, format!("unexpected {token} directive"));
                    }

                    self.push_back(raw);
                    closed = true;
                    break;
                },
                _ => {},
            }
        }

        if !closed {
            let location = self.last_if;
            self.diagnose(Code::BadDirective, location, "missing #endif directive");
        }

        self.carried_layout = carried;
        self.line_layout = line;
        self.line_indent = line_indent;
        self.can_use_directive = true;

        if let Some((stack_depth, indent)) = indent
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_indent(indent);
        }

        if let Some((stack_depth, nesting)) = nesting
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.restore_nesting(nesting);
        }

        if let Some((stack_depth, count)) = errors
            && stack_depth == self.include_stack.len()
            && let Some(state) = self.include_stack.last_mut()
        {
            state.lexer.truncate_errors(count);
        }
    }

    /// `#define M(x) x`
    fn try_macro(&mut self, name: &str, invocation: RawToken<'a>) -> bool {
        if invocation.hidden.contains(name) {
            return false;
        }

        let Some(define) = self.defines.lookup(name) else {
            return false;
        };

        let arguments = if define.is_function_like() {
            match self.collect_arguments() {
                Some(arguments) => Some(arguments),
                None => return false,
            }
        } else {
            None
        };

        let tokens = self.expand(&define, arguments.as_deref(), invocation);
        self.expansions.push(Expansion { tokens });

        true
    }

    /// `M(a, b)`
    fn collect_arguments(&mut self) -> Option<Vec<Vec<RawToken<'a>>>> {
        let open = self.next_significant()?;
        if open.token != Token::ParenLeft {
            self.push_back(open);
            return None;
        }

        let mut arguments = Vec::new();
        let mut current: Vec<RawToken<'a>> = Vec::new();
        let mut depth = 1usize;

        loop {
            let Some(raw) = self.next_raw() else {
                self.diagnose(Code::BadDirective, open.location, "missing ')' in macro call");
                break;
            };
            let token = raw.token;

            match token {
                Token::Comma if depth == 1 => arguments.push(std::mem::take(&mut current)),
                Token::ParenRight => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }

                    current.push(raw);
                },
                Token::ParenLeft => {
                    depth += 1;
                    current.push(raw);
                },
                Token::Newline | Token::SuppressedNewline => {},
                token if token.is_comment() => {},
                _ => current.push(raw),
            }
        }

        arguments.push(current);

        Some(arguments)
    }

    fn expand(
        &mut self, define: &Define<'a>, arguments: Option<&[Vec<RawToken<'a>>]>, invocation: RawToken<'a>,
    ) -> VecDeque<RawToken<'a>> {
        let fixed_hidden = invocation.hidden.with(&define.name);
        if let Some(builtin) = define.builtin {
            return VecDeque::from([RawToken {
                token: self.expand_builtin(builtin, invocation.location),
                hidden: fixed_hidden,
                ..invocation
            }]);
        }

        let mut out: Vec<(Token<'a>, bool, HideSet)> = Vec::new();
        // `foo##x`, `foo x`
        let mut separated = true;

        for part in &define.body {
            match part {
                BodyPart::Space => separated = true,
                BodyPart::Stringify(name) => {
                    let text = match arguments.zip(define.lookup(name)) {
                        Some((arguments, slot)) => {
                            self.render_slot(slot, arguments).unwrap_or_else(|| name.to_string())
                        },
                        None => name.to_string(),
                    };

                    out.push((
                        Token::RawStringLiteral(self.arena.alloc(text)),
                        separated,
                        fixed_hidden.clone(),
                    ));
                    separated = false;
                },
                BodyPart::Paste(name) => {
                    let tokens = Self::slot_tokens(arguments, define, name)
                        .unwrap_or_else(|| vec![(Token::from_identifier(name), false, fixed_hidden.clone())]);
                    self.splice(&mut out, &tokens, false);

                    separated = false;
                },
                BodyPart::Token(token) => {
                    match token.word().and_then(|word| Self::slot_tokens(arguments, define, word)) {
                        Some(tokens) => self.splice(&mut out, &tokens, separated),
                        None => out.push((*token, separated, fixed_hidden.clone())),
                    }

                    separated = false;
                },
            }
        }

        out.into_iter()
            .enumerate()
            .map(|(index, (token, separated, hidden))| {
                if index == 0 {
                    RawToken {
                        token,
                        hidden,
                        ..invocation.clone()
                    }
                } else {
                    RawToken::expanded(token, invocation.location, separated, hidden)
                }
            })
            .collect()
    }

    fn expand_builtin(&self, builtin: Builtin, location: Location) -> Token<'a> {
        match builtin {
            Builtin::Line => Token::IntegerLiteral(self.arena.alloc(location.begin.line.to_string())),
            Builtin::File => {
                let path = self
                    .sources
                    .path(location.file)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();

                Token::RawStringLiteral(self.arena.alloc(path))
            },
        }
    }

    /// `#define M(a, rest...)`
    fn slot_tokens(
        arguments: Option<&[Vec<RawToken<'a>>]>, define: &Define<'a>, name: &str,
    ) -> Option<Vec<(Token<'a>, bool, HideSet)>> {
        let (arguments, slot) = arguments.zip(define.lookup(name))?;
        let mut out = Vec::new();

        match slot {
            Slot::Positional(index) => {
                let argument = arguments.get(index)?;
                push_argument(&mut out, argument, false);
            },
            Slot::Rest(index) => {
                for (offset, argument) in arguments.iter().skip(index).enumerate() {
                    if offset > 0 {
                        out.push((Token::Comma, true, HideSet::default()));
                    }

                    push_argument(&mut out, argument, offset > 0);
                }
            },
        }

        Some(out)
    }

    /// `foo##x`
    fn splice(
        &self, out: &mut Vec<(Token<'a>, bool, HideSet)>, tokens: &[(Token<'a>, bool, HideSet)], mut separated: bool,
    ) {
        for (index, (token, token_separated, hidden)) in tokens.iter().enumerate() {
            if index > 0 {
                separated = *token_separated;
            }

            let pasted = (!separated)
                .then(|| pastable(token).zip(out.last().and_then(|(token, ..)| token.word())))
                .flatten();

            match pasted.zip(out.last_mut()) {
                Some(((right, left), last)) => {
                    let joined = self.arena.alloc(format!("{left}{right}"));
                    last.0 = Token::from_identifier(joined);
                    last.2 = last.2.union(hidden);
                },
                None => out.push((*token, separated, hidden.clone())),
            }
        }
    }

    fn render_slot(&self, slot: Slot, arguments: &[Vec<RawToken<'a>>]) -> Option<String> {
        let mut text = String::new();

        match slot {
            Slot::Positional(index) => {
                for raw in arguments.get(index)? {
                    text.push_str(&raw.token.to_string());
                }
            },
            Slot::Rest(index) => {
                for (offset, argument) in arguments.iter().skip(index).enumerate() {
                    if offset > 0 {
                        text.push(',');
                    }

                    for raw in argument {
                        text.push_str(&raw.token.to_string());
                    }
                }
            },
        }

        Some(text)
    }

    /// `#include "dir\\file.dm"`
    fn resolve_include(&self, raw: &str) -> PathBuf {
        let normalized = raw.replace('\\', "/");
        let base = self
            .include_stack
            .last()
            .map(|state| state.dir.clone())
            .unwrap_or_default();

        base.join(normalized)
    }

    fn include_file(&mut self, path: &Path, location: Location) {
        let path = normalize(path);

        if self.included.contains(&path) {
            self.diagnose(
                Code::FileAlreadyIncluded,
                location,
                format!("file \"{}\" was already included", path.display()),
            );
            return;
        }

        if !path.is_file() {
            self.diagnose(
                Code::MissingIncludedFile,
                location,
                format!("could not find included file \"{}\"", path.display()),
            );
            return;
        }

        self.included.insert(path.clone());

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();

        if matches!(extension.as_str(), "dmm" | "dmp" | "dmf" | "dmi") {
            self.resources.push(path);
            return;
        }

        self.open(&path, location);
    }

    fn open(&mut self, path: &Path, location: Location) {
        let contents = match self.source_cache.files.get(path).copied() {
            Some(contents) => contents,
            None => match std::fs::read_to_string(path) {
                Ok(contents) => {
                    let contents = self.arena.alloc(contents);
                    self.source_cache.files.insert(path.to_path_buf(), contents);
                    contents
                },
                Err(error) => {
                    self.diagnose(
                        Code::MissingIncludedFile,
                        location,
                        format!("could not read \"{}\": {error}", path.display()),
                    );
                    return;
                },
            },
        };
        let file = self.sources.add_borrowed(path, contents);

        if let Some(progress) = self.progress.as_mut()
            && !progress(path)
        {
            self.aborted = true;

            return;
        }

        self.push_file(file, path);
    }

    fn open_embedded(&mut self, name: &str, contents: &'static str) {
        let arena = self.arena;
        let file = self.sources.add(arena, name, contents.to_string());

        self.push_file(file, Path::new(""));
    }

    fn push_file(&mut self, file: FileId, path: &Path) {
        let contents = self.sources.contents(file).unwrap_or_default();
        let mut lexer = Lexer::with_file(contents, file);
        lexer.emit_suppressed_newlines();

        // `#include "inner.dm"`
        lexer.keep_indents_at_eof();

        if let Some(state) = self.include_stack.last() {
            lexer.restore_indent(state.lexer.indent_state().without_errors());

            // `new /datum/tgs_version(\n #include "version.dm"\n)`
            if state.lexer.layout_suppressed() {
                lexer.disable_layout();
            }
        }

        self.include_stack.push(IncludeState {
            lexer,
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            peeked: None,
        });
    }
}

fn push_argument<'a>(out: &mut Vec<(Token<'a>, bool, HideSet)>, argument: &[RawToken<'a>], mut separated: bool) {
    let mut previous: Option<Location> = None;

    for raw in argument {
        if let Some(previous) = previous {
            separated = raw.separated_from(previous);
        }

        out.push((raw.token, separated, raw.hidden.clone()));
        previous = Some(raw.location);
    }
}

/// `./dir/../file.dm`
fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };

    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                out.pop();
            },
            component => out.push(component),
        }
    }

    out
}

/// `DM_STDDEF`
pub const STDDEF_ENV: &str = "DM_STDDEF";

pub const DEMIR_ENV: &str = "DM_DEMIR";

pub use prelude::{CORE_SOURCE, DEMIR_SOURCE, IMGUI_SOURCE, STDDEF_EXT_SOURCE, STDDEF_SOURCE, VERSION_SOURCE};

pub enum PreludeFile {
    Embedded(&'static str, &'static str),
    Disk(PathBuf),
}

pub fn core_files() -> [PreludeFile; 2] {
    [
        PreludeFile::Embedded("<version.dm>", VERSION_SOURCE),
        PreludeFile::Embedded("<core.dm>", CORE_SOURCE),
    ]
}

pub fn prelude_files() -> Vec<PreludeFile> {
    vec![
        env_override(STDDEF_ENV, PreludeFile::Embedded("<stddef.dm>", STDDEF_SOURCE)),
        PreludeFile::Embedded("<stddef_ext.dm>", STDDEF_EXT_SOURCE),
        env_override(DEMIR_ENV, PreludeFile::Embedded("<demir.dm>", DEMIR_SOURCE)),
        PreludeFile::Embedded("<imgui.dm>", IMGUI_SOURCE),
    ]
}

fn env_override(variable: &str, embedded: PreludeFile) -> PreludeFile {
    match std::env::var_os(variable) {
        Some(path) => PreludeFile::Disk(PathBuf::from(path)),
        None => embedded,
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    fn pp(files: &[(&str, &str)]) -> String {
        let (rendered, _) = pp_with_errors(files);

        rendered
    }

    /// [`pp`], also handing back what the run complained about.
    fn pp_with_errors(files: &[(&str, &str)]) -> (String, Vec<String>) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-pp-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");

        for (name, source) in files {
            fs::write(dir.join(name), source).expect("write source");
        }

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .without_core()
            .without_prelude()
            .run(dir.join(files[0].0))
            .expect("preprocess");
        let rendered = render(&result.tokens);
        let errors = result.errors.iter().map(ToString::to_string).collect();
        let _ = fs::remove_dir_all(&dir);

        (rendered, errors)
    }

    fn indent_errors(source: &str) -> Vec<String> {
        let (_, errors) = pp_with_errors(&[("a.dm", source)]);

        errors
            .into_iter()
            .filter(|error| error.contains("inconsistent indentation"))
            .collect()
    }

    #[test]
    fn an_indented_directive_is_not_part_of_the_code_indentation() {
        // The `#endif` lines up with the `#if`, not with anything the code opened.
        assert_eq!(
            indent_errors("\t#if 0\n\t\t#warn skipped\n\t#endif\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            indent_errors("\t#if 1\n\t\t#warn taken\n\t#endif\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_conditional_compiled_out_mid_block_leaves_the_indentation_alone() {
        let source =
            "/proc/a()\n\tif(1)\n#if 0\n\t\tskipped()\n\t\t\tdeeper()\n#endif\n\t\tone()\n\t\t\ttwo()\n\tthree()\n";

        assert_eq!(indent_errors(source), Vec::<String>::new());
    }

    #[test]
    fn a_real_indentation_mistake_is_still_reported_around_directives() {
        // The dedent lands between two open blocks, which no directive explains away.
        assert_eq!(
            indent_errors("/proc/a()\n\tif(1)\n\t\t\tdeep()\n\t\tmiddle()\n").len(),
            1
        );
        assert_eq!(
            indent_errors("/proc/a()\n\tif(1)\n#if 0\n\t\tskipped()\n#endif\n\t\t\tdeep()\n\t\tmiddle()\n").len(),
            1
        );
    }

    #[test]
    fn macro_generated_defines_preserve_function_like_spacing() {
        let rendered = pp(&[(
            "generated.dm",
            concat!(
                "#define DEFINE #define\n",
                "#define MAKE_FUNCTION(_NAME) DEFINE _NAME(x) x\n",
                "#define MAKE_OBJECT(_NAME) DEFINE _NAME (x) x\n",
                "MAKE_FUNCTION(FUNCTION)\n",
                "FUNCTION\n",
                "FUNCTION(value)\n",
                "MAKE_OBJECT(OBJECT)\n",
                "OBJECT\n",
            ),
        )]);

        assert_eq!(rendered, "FUNCTION\nvalue\n( x ) x");
    }

    #[test]
    fn concat_pastes_fixed_text_onto_a_parameter() {
        let rendered = pp(&[(
            "fixed_concat.dm",
            "#define UPDATE(X) src.X ## _standing\nUPDATE(body)\n",
        )]);

        assert_eq!(rendered, "src . body_standing");
    }

    #[test]
    fn a_bare_generated_function_macro_survives_in_a_nested_path_macro() {
        let rendered = pp(&[(
            "nested.dm",
            concat!(
                "#define DEFINE #define\n",
                "#define MAKE(_NAME) /datum/base/##_NAME {} DEFINE _NAME(x) x\n",
                "#define OTHER(_NAME) /datum/other/##_NAME\n",
                "#define WRAP(_PATH, _NAME) _PATH(_NAME) {}\n",
                "MAKE(OFFSETS)\n",
                "WRAP(OTHER, OFFSETS)\n",
            ),
        )]);

        assert_eq!(rendered, "/ datum / base / OFFSETS { } / datum / other / OFFSETS { }");
    }

    #[test]
    fn a_pasted_identity_macro_captures_a_following_argument_list() {
        let direct = pp(&[(
            "identity.dm",
            concat!(
                "#define IDENTITY(x) x\n",
                "#define PATH(_NAME) datum/namespace/##_NAME/##IDENTITY\n",
                "PATH(CHEM)(var/const/REQUEST = 1)\n",
            ),
        )]);
        let nested = pp(&[(
            "nested_identity.dm",
            concat!(
                "#define IDENTITY(x) x\n",
                "#define ADD_1(a) datum/namespace/##a/##IDENTITY\n",
                "#define ADD_2(a, b) datum/namespace/inner/##IDENTITY\n",
                "#define CREATE(a, b) ADD_1(a)(var/##ADD_2(a, b)(##b = 1))\n",
                "CREATE(CHEM, REQUEST)\n",
            ),
        )]);

        assert_eq!(direct, "datum / namespace / CHEM / var / const / REQUEST = 1");
        assert_eq!(
            nested,
            "datum / namespace / CHEM / var / datum / namespace / inner / REQUEST = 1"
        );
    }

    #[test]
    fn recursive_macros_stop_at_the_token_that_already_expanded_them() {
        let rendered = pp(&[(
            "recursive.dm",
            concat!(
                "#define SELF SELF\n",
                "#define LEFT RIGHT\n",
                "#define RIGHT LEFT\n",
                "SELF\n",
                "LEFT\n",
            ),
        )]);

        assert_eq!(rendered, "SELF\nLEFT");
    }

    #[test]
    fn a_condition_inside_parentheses_ends_at_its_physical_line() {
        let rendered = pp(&[(
            "conditional_list.dm",
            concat!(
                "#define CURRENT 1\n",
                "#define EXPECTED 1\n",
                "var/list/items = list(\n",
                "first,\n",
                "#if CURRENT == EXPECTED\n",
                "conditional,\n",
                "#endif\n",
                "last,\n",
                ")\n",
                "var/after = 1\n",
            ),
        )]);

        assert_eq!(
            rendered,
            "var / list / items = list ( first , conditional , last , )\nvar / after = 1"
        );
    }

    #[test]
    fn an_inactive_branch_does_not_close_the_surrounding_expression() {
        let rendered = pp(&[(
            "conditional_close.dm",
            concat!(
                "var/items = list(first,\\\n",
                "#ifdef WINTER\n",
                "coat)\n",
                "#else\n",
                ")\n",
                "#endif\n",
                "var/after = 1\n",
            ),
        )]);

        assert_eq!(rendered, "var / items = list ( first , )\nvar / after = 1");
    }

    #[test]
    fn indented_dead_branch_keeps_layout() {
        let flat = pp(&[(
            "flat.dm",
            "/datum/a\n#ifdef NOPE\n#include \"x.dm\"\n#endif\n/datum/b\n\tvar/y = 2\n",
        )]);
        let indented = pp(&[(
            "indented.dm",
            "/datum/a\n#ifdef NOPE\n\t#include \"x.dm\"\n#endif\n/datum/b\n\tvar/y = 2\n",
        )]);

        assert_eq!(flat, indented);
        assert_eq!(flat, "/ datum / a\n/ datum / b\n> var / y = 2\n<");
    }

    #[test]
    fn an_include_is_indented_against_its_includer() {
        let spliced = pp(&[
            (
                "outer.dm",
                "#define YES\n/datum/a\n\tvar/x = 1\n#ifdef YES\n\t#include \"inner.dm\"\n#endif\n/datum/b\n",
            ),
            ("inner.dm", "/datum/inc\n\tvar/z = 3\n"),
        ]);

        assert_eq!(
            spliced,
            "/ datum / a\n> var / x = 1\n< / datum / inc\n> var / z = 3\n< / datum / b"
        );
    }

    #[test]
    fn an_include_hands_its_open_blocks_back() {
        let trailing = pp(&[
            ("outer.dm", "/datum/a\n\tvar/x = 1\n#include \"tail.dm\"\n/datum/b\n"),
            ("tail.dm", "\tvar/deep = 4\n"),
        ]);
        assert_eq!(trailing, "/ datum / a\n> var / x = 1\nvar / deep = 4\n< / datum / b");

        let empty = pp(&[
            (
                "block.dm",
                "/datum/a\n\tvar/x = 1\n\t#include \"nothing.dm\"\n\tvar/y = 2\n",
            ),
            ("nothing.dm", ""),
        ]);
        assert_eq!(empty, "/ datum / a\n> var / x = 1\nvar / y = 2\n<");
    }

    #[test]
    fn source_text_can_be_shared_between_configuration_passes() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("demir-source-cache-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let entry = dir.join("entry.dm");
        fs::write(&entry, "/datum/a\n\tvar/x = 1\n").expect("first source");

        let arena = StrArena::new();
        let first = Preprocessor::new(&arena)
            .without_core()
            .without_prelude()
            .run(&entry)
            .expect("first pass");
        let cache = first.source_cache.clone();
        assert_eq!(cache.len(), 1);

        fs::write(&entry, "/datum/a\n\tvar/x = 2\n").expect("changed source");
        let second = Preprocessor::new(&arena)
            .without_core()
            .without_prelude()
            .with_source_cache(cache)
            .run(&entry)
            .expect("second pass");
        fs::remove_dir_all(&dir).expect("remove temp dir");

        assert!(render(&second.tokens).contains("x = 1"));
        assert!(!render(&second.tokens).contains("x = 2"));
    }

    #[test]
    fn an_included_file_cannot_leak_a_peeked_eof() {
        let rendered = pp(&[
            ("outer.dm", "/datum/a\n\tvar/x = 1\n#include \"tail.dm\"\n/datum/b\n"),
            ("tail.dm", "\tvar/deep = LAST"),
        ]);

        assert_eq!(rendered, "/ datum / a\n> var / x = 1\nvar / deep = LAST\n< / datum / b");
    }

    /// `stddef.dm`, then `demir.dm`, then the entry. The second prelude file may lean on the first.
    #[test]
    fn the_prelude_runs_in_order_ahead_of_the_entry() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-prelude-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");

        fs::write(dir.join("first.dm"), "#define FROM_FIRST 1\n").expect("write first");
        fs::write(
            dir.join("second.dm"),
            "#if defined(FROM_FIRST)\n#define ORDER 1\n#else\n#define ORDER 0\n#endif\n",
        )
        .expect("write second");
        fs::write(dir.join("entry.dme"), "/datum/a\n\tvar/x = ORDER\n").expect("write entry");

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .with_prelude([
                PreludeFile::Disk(dir.join("first.dm")),
                PreludeFile::Disk(dir.join("second.dm")),
            ])
            .run(dir.join("entry.dme"))
            .expect("preprocess");

        let rendered = render(&result.tokens);
        let _ = fs::remove_dir_all(&dir);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(rendered.contains("x = 1"), "{rendered}");
    }

    #[test]
    fn the_postlude_runs_after_the_entry_and_shares_its_defines() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-postlude-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(
            dir.join("entry.dm"),
            "#define FROM_ENTRY 7\n/datum/entry\n\tvar/value = FROM_PRELUDE\n",
        )
        .expect("write entry");

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .without_prelude()
            .with_prelude([PreludeFile::Embedded("<before-entry.dm>", "#define FROM_PRELUDE 3\n")])
            .with_postlude([PreludeFile::Embedded(
                "<after-entry.dm>",
                "/datum/injected\n\tvar/value = FROM_ENTRY\n",
            )])
            .run(dir.join("entry.dm"))
            .expect("preprocess");

        let rendered = render(&result.tokens);
        let _ = fs::remove_dir_all(&dir);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(rendered.contains("/ datum / entry\n> var / value = 3"), "{rendered}");
        assert!(rendered.contains("/ datum / injected\n> var / value = 7"), "{rendered}");
    }

    #[test]
    fn baking_disables_mapping_compatibility_defines() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-baking-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let entry = dir.join("entry.dm");
        // tgstation's `__byond_version_compat.dm`, which only `SPACEMAN_DMM` used to skip
        fs::write(
            &entry,
            "#if (DM_VERSION < 516 || DM_BUILD < 1659) && !defined(SPACEMAN_DMM)\n#error too \
             old\n#endif\n/proc/test()\n\treturn 1\n",
        )
        .expect("write entry");

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .with_baking(true)
            .run(&entry)
            .expect("preprocess");
        let _ = fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "{:?}", result.errors);
        assert!(result.defines.is_defined("__DEMIR_BAKE__"));
        assert!(!result.defines.is_defined("FASTDMM"));
        assert!(!result.defines.is_defined("SPACEMAN_DMM"));
    }

    /// `#if 0` around code the lexer would reject must not diagnose the dead branch. Indentation
    /// errors already ride along in `IndentState`; these are the ones that do not.
    #[test]
    fn inactive_branch_lex_errors_do_not_leak() {
        let dir = std::env::temp_dir().join(format!("demir-inactive-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let entry = dir.join("entry.dm");
        fs::write(
            &entry,
            "#if 0\n/proc/ignored()\n\tvar/x = `\n\tvar/y = \"unterminated\n#endif\n/proc/valid()\n\treturn 1\n",
        )
        .expect("write entry");

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .without_core()
            .without_prelude()
            .run(&entry)
            .expect("preprocess");
        let rendered = render(&result.tokens);
        let _ = fs::remove_dir_all(&dir);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(!rendered.contains("ignored"), "{rendered}");
        assert!(rendered.contains("valid"), "{rendered}");
    }

    /// Every source is compiled in, so a shipped binary needs no prelude directory beside it.
    #[test]
    fn the_default_prelude_is_version_core_stddef_ext_demir_then_imgui_and_needs_no_files() {
        let names: Vec<_> = core_files()
            .into_iter()
            .chain(prelude_files())
            .map(|file| match file {
                PreludeFile::Embedded(name, _) => name,
                PreludeFile::Disk(_) => "disk",
            })
            .collect();

        assert_eq!(
            names,
            vec![
                "<version.dm>",
                "<core.dm>",
                "<stddef.dm>",
                "<stddef_ext.dm>",
                "<demir.dm>",
                "<imgui.dm>"
            ]
        );
        assert!(CORE_SOURCE.contains("/datum"));
        assert!(STDDEF_SOURCE.contains("#define NORTH 1"));
        assert!(STDDEF_EXT_SOURCE.contains("/atom"));
        assert!(DEMIR_SOURCE.contains("#define __DEMIR__"));
    }

    #[test]
    fn disabling_the_optional_prelude_keeps_core() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-core-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let entry = dir.join("entry.dm");
        fs::write(&entry, "/proc/test()\n\treturn 1\n").expect("write entry");

        let arena = StrArena::new();
        let result = Preprocessor::new(&arena)
            .without_prelude()
            .run(&entry)
            .expect("preprocess");
        let _ = fs::remove_dir_all(&dir);

        assert!(result.sources.find("<version.dm>").is_some());
        assert!(result.sources.find("<core.dm>").is_some());
        assert!(result.sources.find("<stddef.dm>").is_none());
        assert!(result.sources.find("<demir.dm>").is_none());
        assert!(result.defines.is_defined("DM_VERSION"));
        assert!(result.defines.is_defined("DM_BUILD"));
    }

    /// The builtins reach the tree without a single file read.
    #[test]
    fn the_embedded_prelude_defines_the_builtins() {
        let arena = StrArena::new();
        let mut preprocessor = Preprocessor::new(&arena);
        preprocessor.prelude = vec![
            PreludeFile::Embedded("<version.dm>", VERSION_SOURCE),
            PreludeFile::Embedded("<core.dm>", CORE_SOURCE),
            PreludeFile::Embedded("<stddef.dm>", STDDEF_SOURCE),
            PreludeFile::Embedded("<stddef_ext.dm>", STDDEF_EXT_SOURCE),
            PreludeFile::Embedded("<demir.dm>", DEMIR_SOURCE),
            PreludeFile::Embedded("<imgui.dm>", IMGUI_SOURCE),
        ];

        for file in std::mem::take(&mut preprocessor.prelude) {
            let PreludeFile::Embedded(name, contents) = file else {
                continue;
            };

            preprocessor.open_embedded(name, contents);
            preprocessor.drain();
            preprocessor.end_stream();
        }

        assert!(preprocessor.errors.is_empty(), "{:?}", preprocessor.errors);
        assert!(preprocessor.defines.is_defined("NORTH"));
        assert!(preprocessor.defines.is_defined("DM_VERSION"));
    }
}
