use std::collections::HashMap;

/// OpenDream `WarningCode` numbers
/// The ODXXXX error codes are becoming kinda standard, and I like them more so always keep this
/// 1:1 with OpenDream error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum Code {
    BadToken = 1,
    BadDirective = 10,
    InvalidInclusion = 900,

    FileAlreadyIncluded = 1000,
    MissingIncludedFile = 1001,
    InvalidWarningCode = 1002,
    InvalidFileDirDefine = 1003,
    MisplacedDirective = 1100,
    UndefineMissingDirective = 1101,
    DefinedMissingParen = 1150,
    ErrorDirective = 1200,
    WarningDirective = 1201,
    MiscapitalizedDirective = 1300,

    SoftReservedKeyword = 2000,
}

impl Code {
    pub fn number(self) -> u16 { self as u16 }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "BadToken" => Self::BadToken,
            "BadDirective" => Self::BadDirective,
            "InvalidInclusion" => Self::InvalidInclusion,
            "FileAlreadyIncluded" => Self::FileAlreadyIncluded,
            "MissingIncludedFile" => Self::MissingIncludedFile,
            "InvalidWarningCode" => Self::InvalidWarningCode,
            "InvalidFileDirDefine" => Self::InvalidFileDirDefine,
            "MisplacedDirective" => Self::MisplacedDirective,
            "UndefineMissingDirective" => Self::UndefineMissingDirective,
            "DefinedMissingParen" => Self::DefinedMissingParen,
            "ErrorDirective" => Self::ErrorDirective,
            "WarningDirective" => Self::WarningDirective,
            "MiscapitalizedDirective" => Self::MiscapitalizedDirective,
            "SoftReservedKeyword" => Self::SoftReservedKeyword,
            _ => return None,
        })
    }

    pub fn from_number(number: u16) -> Option<Self> {
        const ALL: &[Code] = &[
            Code::BadToken,
            Code::BadDirective,
            Code::InvalidInclusion,
            Code::FileAlreadyIncluded,
            Code::MissingIncludedFile,
            Code::InvalidWarningCode,
            Code::InvalidFileDirDefine,
            Code::MisplacedDirective,
            Code::UndefineMissingDirective,
            Code::DefinedMissingParen,
            Code::ErrorDirective,
            Code::WarningDirective,
            Code::MiscapitalizedDirective,
            Code::SoftReservedKeyword,
        ];

        ALL.iter().copied().find(|c| c.number() == number)
    }

    pub fn default_level(self) -> Level {
        match self {
            Self::BadToken | Self::BadDirective | Self::InvalidInclusion => Level::Error,
            Self::MissingIncludedFile | Self::MisplacedDirective | Self::ErrorDirective => Level::Error,
            Self::FileAlreadyIncluded
            | Self::InvalidWarningCode
            | Self::InvalidFileDirDefine
            | Self::UndefineMissingDirective
            | Self::DefinedMissingParen
            | Self::WarningDirective
            | Self::MiscapitalizedDirective
            | Self::SoftReservedKeyword => Level::Warning,
        }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "OD{:04}", self.number()) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Disabled,
    Notice,
    Warning,
    Error,
}

impl Level {
    pub fn from_pragma(word: &str) -> Option<Self> {
        Some(match word {
            "disabled" | "disable" => Self::Disabled,
            "notice" | "pedantic" | "info" => Self::Notice,
            "warning" | "warn" => Self::Warning,
            "error" | "err" => Self::Error,
            _ => return None,
        })
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Disabled => "disabled",
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::Error => "error",
        };

        f.write_str(text)
    }
}

/// `#pragma`
#[derive(Default)]
pub struct PragmaTable {
    overrides: HashMap<Code, Level>,
}

impl PragmaTable {
    pub fn new() -> Self { Self::default() }

    pub fn level(&self, code: Code) -> Level {
        self.overrides
            .get(&code)
            .copied()
            .unwrap_or_else(|| code.default_level())
    }

    /// Codes below `1000` stay fatal
    pub fn set(&mut self, code: Code, level: Level) -> bool {
        if code.number() < 1000 {
            return false;
        }

        self.overrides.insert(code, level);

        true
    }
}
