#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoftKeyword {
    Args,
    Callee,
    Caller,
    Const,
    Final,
    Global,
    Input,
    List,
    Operator,
    Pick,
    Proc,
    Src,
    Static,
    Step,
    Tmp,
    Usr,
    Var,
    Verb,
    World,
}

impl SoftKeyword {
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "args" => SoftKeyword::Args,
            "callee" => SoftKeyword::Callee,
            "caller" => SoftKeyword::Caller,
            "const" => SoftKeyword::Const,
            "final" => SoftKeyword::Final,
            "global" => SoftKeyword::Global,
            "input" => SoftKeyword::Input,
            "list" => SoftKeyword::List,
            "operator" => SoftKeyword::Operator,
            "pick" => SoftKeyword::Pick,
            "proc" => SoftKeyword::Proc,
            "src" => SoftKeyword::Src,
            "static" => SoftKeyword::Static,
            "step" => SoftKeyword::Step,
            "tmp" => SoftKeyword::Tmp,
            "usr" => SoftKeyword::Usr,
            "var" => SoftKeyword::Var,
            "verb" => SoftKeyword::Verb,
            "world" => SoftKeyword::World,
            _ => return None,
        })
    }

    pub fn as_word(self) -> &'static str {
        match self {
            SoftKeyword::Args => "args",
            SoftKeyword::Callee => "callee",
            SoftKeyword::Caller => "caller",
            SoftKeyword::Const => "const",
            SoftKeyword::Final => "final",
            SoftKeyword::Global => "global",
            SoftKeyword::Input => "input",
            SoftKeyword::List => "list",
            SoftKeyword::Operator => "operator",
            SoftKeyword::Pick => "pick",
            SoftKeyword::Proc => "proc",
            SoftKeyword::Src => "src",
            SoftKeyword::Static => "static",
            SoftKeyword::Step => "step",
            SoftKeyword::Tmp => "tmp",
            SoftKeyword::Usr => "usr",
            SoftKeyword::Var => "var",
            SoftKeyword::Verb => "verb",
            SoftKeyword::World => "world",
        }
    }
}

impl std::fmt::Display for SoftKeyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.as_word()) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    Eof,
    Identifier(&'a str),
    Soft(SoftKeyword),

    Newline,
    SuppressedNewline,
    Indent,
    Dedent,

    As,
    Break,
    Catch,
    Continue,
    Del,
    Do,
    Else,
    For,
    Goto,
    If,
    In,
    New,
    Null,
    Return,
    Set,
    Spawn,
    Switch,
    Throw,
    To,
    Try,
    While,

    PpDefine,
    PpElif,
    PpElse,
    PpEndif,
    PpIf,
    PpIfdef,
    PpIfndef,
    PpInclude,
    PpPragma,
    PpUndef,
    PpError(&'a str),
    PpWarn(&'a str),

    Equal,       // =
    Add,         // +
    Sub,         // -
    Mul,         // *
    Slash,       // /
    Modulo,      // %
    Comma,       // ,
    Dot,         // .
    Colon,       // :
    Semicolon,   // ;
    Backslash,   // \
    Exclaim,     // !
    Question,    // ?
    Hash,        // #
    At,          // @
    ParenLeft,   // (
    ParenRight,  // )
    BraceLeft,   // {
    BraceRight,  // }
    SquareLeft,  // [
    SquareRight, // ]
    AngleLeft,   // <
    AngleRight,  // >
    BitAnd,      // &
    BitXor,      // ^
    BitOr,       // |
    BitNot,      // ~

    Pow,                // **
    AddEqual,           // +=
    SubEqual,           // -=
    MulEqual,           // *=
    DivEqual,           // /=
    ModEqual,           // %=
    FloatModulo,        // %%
    FloatModuloEqual,   // %%=
    PowEqual,           // **=
    AndEqual,           // &=
    OrEqual,            // |=
    XorEqual,           // ^=
    LogicalAndEqual,    // &&=
    LogicalOrEqual,     // ||=
    ShiftLeft,          // <<
    ShiftRight,         // >>
    ShiftLeftEqual,     // <<=
    ShiftRightEqual,    // >>=
    LogicalAnd,         // &&
    LogicalOr,          // ||
    CompareEqual,       // ==
    CompareNotEqual,    // !=
    CompareNotEqualAlt, // <>
    CompareThreeWay,    // <=>
    Equivalent,         // ~=
    NotEquivalent,      // ~!
    LessEqual,          // <=
    GreaterEqual,       // >=
    Increment,          // ++
    Decrement,          // --
    Super,              // ..
    Ellipsis,           // ...
    Scope,              // ::
    AssignInto,         // :=
    SafeDot,            // ?.
    SafeColon,          // ?:
    SafeSquare,         // ?[
    Concat,             // ##
    Repeat,             // ###

    IntegerLiteral(&'a str),
    FloatingPointLiteral(&'a str),
    HexLiteral(&'a str),
    StringLiteral(&'a str),
    RawStringLiteral(&'a str),
    ResourceLiteral(&'a str),

    InterpStringBegin(&'a str),
    InterpStringMid(&'a str),
    InterpStringEnd(&'a str),

    LineComment(&'a str),
    BlockComment(&'a str),
}

impl<'a> Token<'a> {
    pub fn from_identifier(ident: &'a str) -> Self {
        match ident {
            "as" => Token::As,
            "break" => Token::Break,
            "catch" => Token::Catch,
            "continue" => Token::Continue,
            "del" => Token::Del,
            "do" => Token::Do,
            "else" => Token::Else,
            "for" => Token::For,
            "goto" => Token::Goto,
            "if" => Token::If,
            "in" => Token::In,
            "new" => Token::New,
            "null" => Token::Null,
            "return" => Token::Return,
            "set" => Token::Set,
            "spawn" => Token::Spawn,
            "switch" => Token::Switch,
            "throw" => Token::Throw,
            "to" => Token::To,
            "try" => Token::Try,
            "while" => Token::While,
            _ => match SoftKeyword::from_word(ident) {
                Some(keyword) => Token::Soft(keyword),
                None => Token::Identifier(ident),
            },
        }
    }

    /// `#error`/`#warn` take rest of the line so we dont include them here
    pub fn from_directive(ident: &'a str) -> Option<Self> {
        Some(match ident {
            "define" => Token::PpDefine,
            "elif" => Token::PpElif,
            "else" => Token::PpElse,
            "endif" => Token::PpEndif,
            "if" => Token::PpIf,
            "ifdef" => Token::PpIfdef,
            "ifndef" => Token::PpIfndef,
            "include" => Token::PpInclude,
            "pragma" => Token::PpPragma,
            "undef" => Token::PpUndef,
            _ => return None,
        })
    }

    pub fn word(&self) -> Option<&'a str> {
        Some(match self {
            Token::Identifier(s) => s,
            Token::Soft(keyword) => keyword.as_word(),
            Token::As => "as",
            Token::Break => "break",
            Token::Catch => "catch",
            Token::Continue => "continue",
            Token::Del => "del",
            Token::Do => "do",
            Token::Else => "else",
            Token::For => "for",
            Token::Goto => "goto",
            Token::If => "if",
            Token::In => "in",
            Token::New => "new",
            Token::Null => "null",
            Token::Return => "return",
            Token::Set => "set",
            Token::Spawn => "spawn",
            Token::Switch => "switch",
            Token::Throw => "throw",
            Token::To => "to",
            Token::Try => "try",
            Token::While => "while",
            _ => return None,
        })
    }

    pub fn is_comment(&self) -> bool { matches!(self, Token::LineComment(_) | Token::BlockComment(_)) }

    pub fn is_identifier(&self) -> bool { self.identifier_name().is_some() }

    /// The name behind a token that can stand in for an ordinary identifier. Soft keywords do that
    /// everywhere outside the position that reserves them.
    pub fn identifier_name(&self) -> Option<&'a str> {
        match self {
            Token::Identifier(s) => Some(s),
            Token::Soft(keyword) => Some(keyword.as_word()),
            _ => None,
        }
    }

    pub fn is_layout(&self) -> bool {
        matches!(
            self,
            Token::Newline | Token::SuppressedNewline | Token::Indent | Token::Dedent
        )
    }

    pub fn is_directive(&self) -> bool {
        matches!(
            self,
            Token::PpDefine
                | Token::PpElif
                | Token::PpElse
                | Token::PpEndif
                | Token::PpError(_)
                | Token::PpIf
                | Token::PpIfdef
                | Token::PpIfndef
                | Token::PpInclude
                | Token::PpPragma
                | Token::PpUndef
                | Token::PpWarn(_)
        )
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Token::IntegerLiteral(_)
                | Token::FloatingPointLiteral(_)
                | Token::HexLiteral(_)
                | Token::StringLiteral(_)
                | Token::RawStringLiteral(_)
                | Token::ResourceLiteral(_)
                | Token::InterpStringBegin(_)
                | Token::InterpStringMid(_)
                | Token::InterpStringEnd(_)
        )
    }

    pub fn text(&self) -> Option<&'a str> {
        match self {
            Token::Identifier(s)
            | Token::IntegerLiteral(s)
            | Token::FloatingPointLiteral(s)
            | Token::HexLiteral(s)
            | Token::StringLiteral(s)
            | Token::RawStringLiteral(s)
            | Token::ResourceLiteral(s)
            | Token::InterpStringBegin(s)
            | Token::InterpStringMid(s)
            | Token::InterpStringEnd(s)
            | Token::LineComment(s)
            | Token::BlockComment(s) => Some(s),
            _ => None,
        }
    }
}

impl std::fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Eof => write!(f, "eof"),
            Token::Identifier(s) => write!(f, "{s}"),
            Token::Soft(keyword) => write!(f, "{}", keyword.as_word()),

            Token::Newline => write!(f, "newline"),
            Token::SuppressedNewline => write!(f, "suppressed newline"),
            Token::Indent => write!(f, "indent"),
            Token::Dedent => write!(f, "dedent"),

            Token::As => write!(f, "as"),
            Token::Break => write!(f, "break"),
            Token::Catch => write!(f, "catch"),
            Token::Continue => write!(f, "continue"),
            Token::Del => write!(f, "del"),
            Token::Do => write!(f, "do"),
            Token::Else => write!(f, "else"),
            Token::For => write!(f, "for"),
            Token::Goto => write!(f, "goto"),
            Token::If => write!(f, "if"),
            Token::In => write!(f, "in"),
            Token::New => write!(f, "new"),
            Token::Null => write!(f, "null"),
            Token::Return => write!(f, "return"),
            Token::Set => write!(f, "set"),
            Token::Spawn => write!(f, "spawn"),
            Token::Switch => write!(f, "switch"),
            Token::Throw => write!(f, "throw"),
            Token::To => write!(f, "to"),
            Token::Try => write!(f, "try"),
            Token::While => write!(f, "while"),

            Token::PpDefine => write!(f, "#define"),
            Token::PpElif => write!(f, "#elif"),
            Token::PpElse => write!(f, "#else"),
            Token::PpEndif => write!(f, "#endif"),
            Token::PpError(s) => write!(f, "#error{s}"),
            Token::PpIf => write!(f, "#if"),
            Token::PpIfdef => write!(f, "#ifdef"),
            Token::PpIfndef => write!(f, "#ifndef"),
            Token::PpInclude => write!(f, "#include"),
            Token::PpPragma => write!(f, "#pragma"),
            Token::PpUndef => write!(f, "#undef"),
            Token::PpWarn(s) => write!(f, "#warn{s}"),

            Token::Equal => write!(f, "="),
            Token::Add => write!(f, "+"),
            Token::Sub => write!(f, "-"),
            Token::Mul => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Modulo => write!(f, "%"),
            Token::Comma => write!(f, ","),
            Token::Dot => write!(f, "."),
            Token::Colon => write!(f, ":"),
            Token::Semicolon => write!(f, ";"),
            Token::Backslash => write!(f, "\\"),
            Token::Exclaim => write!(f, "!"),
            Token::Question => write!(f, "?"),
            Token::Hash => write!(f, "#"),
            Token::At => write!(f, "@"),
            Token::ParenLeft => write!(f, "("),
            Token::ParenRight => write!(f, ")"),
            Token::BraceLeft => write!(f, "{{"),
            Token::BraceRight => write!(f, "}}"),
            Token::SquareLeft => write!(f, "["),
            Token::SquareRight => write!(f, "]"),
            Token::AngleLeft => write!(f, "<"),
            Token::AngleRight => write!(f, ">"),
            Token::BitAnd => write!(f, "&"),
            Token::BitXor => write!(f, "^"),
            Token::BitOr => write!(f, "|"),
            Token::BitNot => write!(f, "~"),

            Token::Pow => write!(f, "**"),
            Token::AddEqual => write!(f, "+="),
            Token::SubEqual => write!(f, "-="),
            Token::MulEqual => write!(f, "*="),
            Token::DivEqual => write!(f, "/="),
            Token::ModEqual => write!(f, "%="),
            Token::FloatModulo => write!(f, "%%"),
            Token::FloatModuloEqual => write!(f, "%%="),
            Token::PowEqual => write!(f, "**="),
            Token::AndEqual => write!(f, "&="),
            Token::OrEqual => write!(f, "|="),
            Token::XorEqual => write!(f, "^="),
            Token::LogicalAndEqual => write!(f, "&&="),
            Token::LogicalOrEqual => write!(f, "||="),
            Token::ShiftLeft => write!(f, "<<"),
            Token::ShiftRight => write!(f, ">>"),
            Token::ShiftLeftEqual => write!(f, "<<="),
            Token::ShiftRightEqual => write!(f, ">>="),
            Token::LogicalAnd => write!(f, "&&"),
            Token::LogicalOr => write!(f, "||"),
            Token::CompareEqual => write!(f, "=="),
            Token::CompareNotEqual => write!(f, "!="),
            Token::CompareNotEqualAlt => write!(f, "<>"),
            Token::CompareThreeWay => write!(f, "<=>"),
            Token::Equivalent => write!(f, "~="),
            Token::NotEquivalent => write!(f, "~!"),
            Token::LessEqual => write!(f, "<="),
            Token::GreaterEqual => write!(f, ">="),
            Token::Increment => write!(f, "++"),
            Token::Decrement => write!(f, "--"),
            Token::Super => write!(f, ".."),
            Token::Ellipsis => write!(f, "..."),
            Token::Scope => write!(f, "::"),
            Token::AssignInto => write!(f, ":="),
            Token::SafeDot => write!(f, "?."),
            Token::SafeColon => write!(f, "?:"),
            Token::SafeSquare => write!(f, "?["),
            Token::Concat => write!(f, "##"),
            Token::Repeat => write!(f, "###"),

            Token::IntegerLiteral(s) | Token::FloatingPointLiteral(s) | Token::HexLiteral(s) => write!(f, "{s}"),
            Token::StringLiteral(s) => write!(f, "\"{s}\""),
            Token::RawStringLiteral(s) => write!(f, "@\"{s}\""),
            Token::ResourceLiteral(s) => write!(f, "'{s}'"),
            Token::InterpStringBegin(s) => write!(f, "\"{s}["),
            Token::InterpStringMid(s) => write!(f, "]{s}["),
            Token::InterpStringEnd(s) => write!(f, "]{s}\""),

            Token::LineComment(s) | Token::BlockComment(s) => write!(f, "{s}"),
        }
    }
}
