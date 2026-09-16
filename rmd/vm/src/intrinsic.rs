use core::{path::TreePath, types::Identifier};
use std::path::PathBuf;

use crate::{
    FaultKind,
    GenericValue,
    builtins::index_arg,
    eval::Evaluator,
    heap::ObjectId,
    value::{ListData, Receiver},
    world::Position,
};

type Result<T> = std::result::Result<T, crate::Fault>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intrinsic {
    WorldNew,
    WorldDel,
    WorldTopic,
    WorldError,
    WorldReboot,
    WorldRepop,
    WorldExport,
    WorldImport,
    WorldOpenPort,
    WorldProfile,
    WorldGetConfig,
    WorldSetConfig,
    WorldIsBanned,
    WorldIsSubscribed,
    WorldAddCredits,
    WorldGetCredits,
    WorldPayCredits,
    WorldGetScores,
    WorldSetScores,
    WorldGetMedal,
    WorldSetMedal,
    WorldClearMedal,
    WorldFile2List,
    Abs,
    Addtext,
    Alert,
    Alist,
    Animate,
    Arccos,
    Arcsin,
    Arctan,
    Ascii2text,
    Astype,
    Block,
    BoundPixloc,
    BoundsDist,
    Bounds,
    Browse,
    BrowseRsc,
    Ceil,
    CkeyEx,
    Ckey,
    Clamp,
    CmptextEx,
    Cmptext,
    CopytextChar,
    Copytext,
    Cos,
    Crash,
    DmDbClose,
    DmDbColumns,
    DmDbConnect,
    DmDbErrorMsg,
    DmDbExecute,
    DmDbIsConnected,
    DmDbNewCon,
    DmDbNewQuery,
    DmDbNextRow,
    DmDbQuote,
    DmDbRowCount,
    DmDbRowsAffected,
    FcopyRsc,
    Fcopy,
    Fdel,
    Fexists,
    File2text,
    File,
    Filter,
    FindlasttextChar,
    FindlasttextExChar,
    FindlasttextEx,
    Findlasttext,
    FindtextChar,
    FindtextExChar,
    FindtextEx,
    Findtext,
    Flick,
    Flist,
    Floor,
    Fract,
    Ftime,
    Ftp,
    GetDir,
    GetDist,
    GetStepAway,
    GetStepRand,
    GetStep,
    GetStepsTo,
    GetStepTo,
    GetStepTowards,
    Generator,
    Gradient,
    Hascall,
    Hearers,
    HtmlDecode,
    HtmlEncode,
    Icon,
    IconStates,
    Image,
    Isarea,
    Isfile,
    Isicon,
    Isinf,
    Islist,
    Isloc,
    Ismob,
    Ismovable,
    Isnan,
    Isnull,
    Isnum,
    Isobj,
    Ispath,
    Ispointer,
    Issaved,
    Istext,
    Isturf,
    Istype,
    Jointext,
    JsonDecode,
    JsonEncode,
    LengthChar,
    Length,
    Lentext,
    Lerp,
    Link,
    List2params,
    LoadExt,
    LoadResource,
    Locate,
    Log,
    Lowertext,
    Matrix,
    Max,
    Md5,
    Min,
    Missile,
    MutableAppearance,
    Newlist,
    NoiseHash,
    NonspantextChar,
    Nameof,
    Nonspantext,
    Num2text,
    Obounds,
    Ohearers,
    Orange,
    Output,
    Oview,
    Oviewers,
    Params2list,
    Pixloc,
    Prob,
    Rand,
    RandSeed,
    Range,
    Ref,
    Refcount,
    Regex,
    ReplacetextChar,
    ReplacetextExChar,
    ReplacetextEx,
    Replacetext,
    Rgb2num,
    Rgb,
    Roll,
    Round,
    Run,
    Sha1,
    Shell,
    Shutdown,
    Sign,
    Sin,
    Sleep,
    SorttextEx,
    Sorttext,
    Sound,
    SpantextChar,
    Spantext,
    SplicetextChar,
    Splicetext,
    SplittextChar,
    Splittext,
    Sqrt,
    Startup,
    Stat,
    Statpanel,
    StepAway,
    StepRand,
    Step,
    StepTo,
    StepTowards,
    Tan,
    Text2asciiChar,
    Text2ascii,
    Text2file,
    Text2num,
    Text2path,
    Text,
    Time2text,
    Trimtext,
    Trunc,
    Turn,
    Typesof,
    Uppertext,
    UrlDecode,
    UrlEncode,
    ValuesCutOver,
    ValuesCutUnder,
    ValuesDot,
    ValuesProduct,
    ValuesSum,
    Vector,
    View,
    Viewers,
    WalkAway,
    WalkRand,
    Walk,
    WalkTo,
    WalkTowards,
    Winclone,
    Winexists,
    Winget,
    Winset,
    Winshow,

    DatumNew,
    DatumDel,
    DatumTopic,
    DatumRead,
    DatumWrite,
    AtomClick,
    AtomDblClick,
    AtomMouseDown,
    AtomMouseDrag,
    AtomMouseDrop,
    AtomMouseEntered,
    AtomMouseExited,
    AtomMouseMove,
    AtomMouseUp,
    AtomMouseWheel,
    AtomEntered,
    AtomExited,
    AtomCrossed,
    AtomUncrossed,
    AtomStat,
    ImageNew,
    MovableBump,
    MovableMove,
    MobLogin,
    MobLogout,
    ClientNew,
    ClientDel,
    ClientTopic,
    ClientStat,
    ClientCommand,
    ClientImport,
    ClientExport,
    ClientAllowUpload,
    ClientSoundQuery,
    ClientMeasureText,
    ClientMove,
    ClientClick,
    ClientDblClick,
    ClientMouseDown,
    ClientMouseDrag,
    ClientMouseDrop,
    ClientMouseEntered,
    ClientMouseExited,
    ClientMouseMove,
    ClientMouseUp,
    ClientMouseWheel,
    ClientIsByondMember,
    ClientCheckPassport,
    ClientSendPage,
    ClientGetAPI,
    ClientSetAPI,
    ClientRenderIcon,
    SavefileNew,
    SavefileFlush,
    SavefileExportText,
    SavefileImportText,
    SavefileLock,
    SavefileUnlock,
    VectorNew,
    VectorCross,
    VectorDot,
    VectorInterpolate,
    VectorNormalize,
    VectorTurn,
    PixlocNew,
    ListAdd,
    ListCopy,
    ListCut,
    ListFind,
    ListInsert,
    ListJoin,
    ListRemove,
    ListRemoveAll,
    ListSwap,
    ListSplice,
    DmNewIcon,
    DmTurnIcon,
    DmFlipIcon,
    DmShiftIcon,
    DmIconIntensity,
    DmIconBlend,
    DmIconSwapColor,
    DmIconDrawBox,
    DmIconInsert,
    DmIconMapColors,
    DmIconScale,
    DmIconCrop,
    DmIconGetPixel,
    DmIconSize,
    DmDatabase,
    MakeGenerator,
}

impl Intrinsic {
    pub const ALL: [Self; 315] = [
        Self::WorldNew,
        Self::WorldDel,
        Self::WorldTopic,
        Self::WorldError,
        Self::WorldReboot,
        Self::WorldRepop,
        Self::WorldExport,
        Self::WorldImport,
        Self::WorldOpenPort,
        Self::WorldProfile,
        Self::WorldGetConfig,
        Self::WorldSetConfig,
        Self::WorldIsBanned,
        Self::WorldIsSubscribed,
        Self::WorldAddCredits,
        Self::WorldGetCredits,
        Self::WorldPayCredits,
        Self::WorldGetScores,
        Self::WorldSetScores,
        Self::WorldGetMedal,
        Self::WorldSetMedal,
        Self::WorldClearMedal,
        Self::WorldFile2List,
        Self::Abs,
        Self::Addtext,
        Self::Alert,
        Self::Alist,
        Self::Animate,
        Self::Arccos,
        Self::Arcsin,
        Self::Arctan,
        Self::Ascii2text,
        Self::Astype,
        Self::Block,
        Self::BoundPixloc,
        Self::BoundsDist,
        Self::Bounds,
        Self::Browse,
        Self::BrowseRsc,
        Self::Ceil,
        Self::CkeyEx,
        Self::Ckey,
        Self::Clamp,
        Self::CmptextEx,
        Self::Cmptext,
        Self::CopytextChar,
        Self::Copytext,
        Self::Cos,
        Self::Crash,
        Self::DmDbClose,
        Self::DmDbColumns,
        Self::DmDbConnect,
        Self::DmDbErrorMsg,
        Self::DmDbExecute,
        Self::DmDbIsConnected,
        Self::DmDbNewCon,
        Self::DmDbNewQuery,
        Self::DmDbNextRow,
        Self::DmDbQuote,
        Self::DmDbRowCount,
        Self::DmDbRowsAffected,
        Self::FcopyRsc,
        Self::Fcopy,
        Self::Fdel,
        Self::Fexists,
        Self::File2text,
        Self::File,
        Self::Filter,
        Self::FindlasttextChar,
        Self::FindlasttextExChar,
        Self::FindlasttextEx,
        Self::Findlasttext,
        Self::FindtextChar,
        Self::FindtextExChar,
        Self::FindtextEx,
        Self::Findtext,
        Self::Flick,
        Self::Flist,
        Self::Floor,
        Self::Fract,
        Self::Ftime,
        Self::Ftp,
        Self::GetDir,
        Self::GetDist,
        Self::GetStepAway,
        Self::GetStepRand,
        Self::GetStep,
        Self::GetStepsTo,
        Self::GetStepTo,
        Self::GetStepTowards,
        Self::Generator,
        Self::Gradient,
        Self::Hascall,
        Self::Hearers,
        Self::HtmlDecode,
        Self::HtmlEncode,
        Self::Icon,
        Self::IconStates,
        Self::Image,
        Self::Isarea,
        Self::Isfile,
        Self::Isicon,
        Self::Isinf,
        Self::Islist,
        Self::Isloc,
        Self::Ismob,
        Self::Ismovable,
        Self::Isnan,
        Self::Isnull,
        Self::Isnum,
        Self::Isobj,
        Self::Ispath,
        Self::Ispointer,
        Self::Issaved,
        Self::Istext,
        Self::Isturf,
        Self::Istype,
        Self::Jointext,
        Self::JsonDecode,
        Self::JsonEncode,
        Self::LengthChar,
        Self::Length,
        Self::Lentext,
        Self::Lerp,
        Self::Link,
        Self::List2params,
        Self::LoadExt,
        Self::LoadResource,
        Self::Locate,
        Self::Log,
        Self::Lowertext,
        Self::Matrix,
        Self::Max,
        Self::Md5,
        Self::Min,
        Self::Missile,
        Self::MutableAppearance,
        Self::Newlist,
        Self::NoiseHash,
        Self::NonspantextChar,
        Self::Nameof,
        Self::Nonspantext,
        Self::Num2text,
        Self::Obounds,
        Self::Ohearers,
        Self::Orange,
        Self::Output,
        Self::Oview,
        Self::Oviewers,
        Self::Params2list,
        Self::Pixloc,
        Self::Prob,
        Self::Rand,
        Self::RandSeed,
        Self::Range,
        Self::Ref,
        Self::Refcount,
        Self::Regex,
        Self::ReplacetextChar,
        Self::ReplacetextExChar,
        Self::ReplacetextEx,
        Self::Replacetext,
        Self::Rgb2num,
        Self::Rgb,
        Self::Roll,
        Self::Round,
        Self::Run,
        Self::Sha1,
        Self::Shell,
        Self::Shutdown,
        Self::Sign,
        Self::Sin,
        Self::Sleep,
        Self::SorttextEx,
        Self::Sorttext,
        Self::Sound,
        Self::SpantextChar,
        Self::Spantext,
        Self::SplicetextChar,
        Self::Splicetext,
        Self::SplittextChar,
        Self::Splittext,
        Self::Sqrt,
        Self::Startup,
        Self::Stat,
        Self::Statpanel,
        Self::StepAway,
        Self::StepRand,
        Self::Step,
        Self::StepTo,
        Self::StepTowards,
        Self::Tan,
        Self::Text2asciiChar,
        Self::Text2ascii,
        Self::Text2file,
        Self::Text2num,
        Self::Text2path,
        Self::Text,
        Self::Time2text,
        Self::Trimtext,
        Self::Trunc,
        Self::Turn,
        Self::Typesof,
        Self::Uppertext,
        Self::UrlDecode,
        Self::UrlEncode,
        Self::ValuesCutOver,
        Self::ValuesCutUnder,
        Self::ValuesDot,
        Self::ValuesProduct,
        Self::ValuesSum,
        Self::Vector,
        Self::View,
        Self::Viewers,
        Self::WalkAway,
        Self::WalkRand,
        Self::Walk,
        Self::WalkTo,
        Self::WalkTowards,
        Self::Winclone,
        Self::Winexists,
        Self::Winget,
        Self::Winset,
        Self::Winshow,
        Self::DatumNew,
        Self::DatumDel,
        Self::DatumTopic,
        Self::DatumRead,
        Self::DatumWrite,
        Self::AtomClick,
        Self::AtomDblClick,
        Self::AtomMouseDown,
        Self::AtomMouseDrag,
        Self::AtomMouseDrop,
        Self::AtomMouseEntered,
        Self::AtomMouseExited,
        Self::AtomMouseMove,
        Self::AtomMouseUp,
        Self::AtomMouseWheel,
        Self::AtomEntered,
        Self::AtomExited,
        Self::AtomCrossed,
        Self::AtomUncrossed,
        Self::AtomStat,
        Self::ImageNew,
        Self::MovableBump,
        Self::MovableMove,
        Self::MobLogin,
        Self::MobLogout,
        Self::ClientNew,
        Self::ClientDel,
        Self::ClientTopic,
        Self::ClientStat,
        Self::ClientCommand,
        Self::ClientImport,
        Self::ClientExport,
        Self::ClientAllowUpload,
        Self::ClientSoundQuery,
        Self::ClientMeasureText,
        Self::ClientMove,
        Self::ClientClick,
        Self::ClientDblClick,
        Self::ClientMouseDown,
        Self::ClientMouseDrag,
        Self::ClientMouseDrop,
        Self::ClientMouseEntered,
        Self::ClientMouseExited,
        Self::ClientMouseMove,
        Self::ClientMouseUp,
        Self::ClientMouseWheel,
        Self::ClientIsByondMember,
        Self::ClientCheckPassport,
        Self::ClientSendPage,
        Self::ClientGetAPI,
        Self::ClientSetAPI,
        Self::ClientRenderIcon,
        Self::SavefileNew,
        Self::SavefileFlush,
        Self::SavefileExportText,
        Self::SavefileImportText,
        Self::SavefileLock,
        Self::SavefileUnlock,
        Self::VectorNew,
        Self::VectorCross,
        Self::VectorDot,
        Self::VectorInterpolate,
        Self::VectorNormalize,
        Self::VectorTurn,
        Self::PixlocNew,
        Self::ListAdd,
        Self::ListCopy,
        Self::ListCut,
        Self::ListFind,
        Self::ListInsert,
        Self::ListJoin,
        Self::ListRemove,
        Self::ListRemoveAll,
        Self::ListSwap,
        Self::ListSplice,
        Self::DmNewIcon,
        Self::DmTurnIcon,
        Self::DmFlipIcon,
        Self::DmShiftIcon,
        Self::DmIconIntensity,
        Self::DmIconBlend,
        Self::DmIconSwapColor,
        Self::DmIconDrawBox,
        Self::DmIconInsert,
        Self::DmIconMapColors,
        Self::DmIconScale,
        Self::DmIconCrop,
        Self::DmIconGetPixel,
        Self::DmIconSize,
        Self::DmDatabase,
        Self::MakeGenerator,
    ];

    pub fn list_proc(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|intrinsic| intrinsic.name().strip_prefix("list.") == Some(name))
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|intrinsic| intrinsic.name() == name)
    }

    pub fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            100 => Self::WorldNew,
            101 => Self::WorldDel,
            102 => Self::WorldTopic,
            103 => Self::WorldError,
            104 => Self::WorldReboot,
            105 => Self::WorldRepop,
            106 => Self::WorldExport,
            107 => Self::WorldImport,
            108 => Self::WorldOpenPort,
            109 => Self::WorldProfile,
            110 => Self::WorldGetConfig,
            111 => Self::WorldSetConfig,
            112 => Self::WorldIsBanned,
            113 => Self::WorldIsSubscribed,
            114 => Self::WorldAddCredits,
            115 => Self::WorldGetCredits,
            116 => Self::WorldPayCredits,
            117 => Self::WorldGetScores,
            118 => Self::WorldSetScores,
            119 => Self::WorldGetMedal,
            120 => Self::WorldSetMedal,
            121 => Self::WorldClearMedal,
            122 => Self::WorldFile2List,
            200 => Self::Abs,
            201 => Self::Addtext,
            202 => Self::Alert,
            203 => Self::Alist,
            204 => Self::Animate,
            205 => Self::Arccos,
            206 => Self::Arcsin,
            207 => Self::Arctan,
            208 => Self::Ascii2text,
            209 => Self::Astype,
            210 => Self::Block,
            211 => Self::BoundPixloc,
            212 => Self::BoundsDist,
            213 => Self::Bounds,
            214 => Self::Browse,
            215 => Self::BrowseRsc,
            216 => Self::Ceil,
            217 => Self::CkeyEx,
            218 => Self::Ckey,
            219 => Self::Clamp,
            220 => Self::CmptextEx,
            221 => Self::Cmptext,
            222 => Self::CopytextChar,
            223 => Self::Copytext,
            224 => Self::Cos,
            225 => Self::Crash,
            226 => Self::DmDbClose,
            227 => Self::DmDbColumns,
            228 => Self::DmDbConnect,
            229 => Self::DmDbErrorMsg,
            230 => Self::DmDbExecute,
            231 => Self::DmDbIsConnected,
            232 => Self::DmDbNewCon,
            233 => Self::DmDbNewQuery,
            234 => Self::DmDbNextRow,
            235 => Self::DmDbQuote,
            236 => Self::DmDbRowCount,
            237 => Self::DmDbRowsAffected,
            238 => Self::FcopyRsc,
            239 => Self::Fcopy,
            240 => Self::Fdel,
            241 => Self::Fexists,
            242 => Self::File2text,
            243 => Self::File,
            244 => Self::Filter,
            245 => Self::FindlasttextChar,
            246 => Self::FindlasttextExChar,
            247 => Self::FindlasttextEx,
            248 => Self::Findlasttext,
            249 => Self::FindtextChar,
            250 => Self::FindtextExChar,
            251 => Self::FindtextEx,
            252 => Self::Findtext,
            253 => Self::Flick,
            254 => Self::Flist,
            255 => Self::Floor,
            256 => Self::Fract,
            257 => Self::Ftime,
            258 => Self::Ftp,
            259 => Self::GetDir,
            260 => Self::GetDist,
            261 => Self::GetStepAway,
            262 => Self::GetStepRand,
            263 => Self::GetStep,
            264 => Self::GetStepsTo,
            265 => Self::GetStepTo,
            266 => Self::GetStepTowards,
            267 => Self::Gradient,
            399 => Self::Generator,
            268 => Self::Hascall,
            269 => Self::Hearers,
            270 => Self::HtmlDecode,
            271 => Self::HtmlEncode,
            272 => Self::Icon,
            273 => Self::IconStates,
            274 => Self::Image,
            275 => Self::Isarea,
            276 => Self::Isfile,
            277 => Self::Isicon,
            278 => Self::Isinf,
            279 => Self::Islist,
            280 => Self::Isloc,
            281 => Self::Ismob,
            282 => Self::Ismovable,
            283 => Self::Isnan,
            284 => Self::Isnull,
            285 => Self::Isnum,
            286 => Self::Isobj,
            287 => Self::Ispath,
            288 => Self::Ispointer,
            289 => Self::Issaved,
            290 => Self::Istext,
            291 => Self::Isturf,
            292 => Self::Istype,
            293 => Self::Jointext,
            294 => Self::JsonDecode,
            295 => Self::JsonEncode,
            296 => Self::LengthChar,
            297 => Self::Length,
            298 => Self::Lentext,
            299 => Self::Lerp,
            300 => Self::Link,
            301 => Self::List2params,
            302 => Self::LoadExt,
            303 => Self::LoadResource,
            304 => Self::Locate,
            305 => Self::Log,
            306 => Self::Lowertext,
            307 => Self::Matrix,
            308 => Self::Max,
            309 => Self::Md5,
            310 => Self::Min,
            311 => Self::Missile,
            312 => Self::MutableAppearance,
            313 => Self::Newlist,
            314 => Self::NoiseHash,
            315 => Self::NonspantextChar,
            316 => Self::Nonspantext,
            400 => Self::Nameof,
            317 => Self::Num2text,
            318 => Self::Obounds,
            319 => Self::Ohearers,
            320 => Self::Orange,
            321 => Self::Output,
            322 => Self::Oview,
            323 => Self::Oviewers,
            324 => Self::Params2list,
            325 => Self::Pixloc,
            326 => Self::Prob,
            327 => Self::Rand,
            328 => Self::RandSeed,
            329 => Self::Range,
            330 => Self::Ref,
            331 => Self::Refcount,
            332 => Self::Regex,
            333 => Self::ReplacetextChar,
            334 => Self::ReplacetextExChar,
            335 => Self::ReplacetextEx,
            336 => Self::Replacetext,
            337 => Self::Rgb2num,
            338 => Self::Rgb,
            339 => Self::Roll,
            340 => Self::Round,
            341 => Self::Run,
            342 => Self::Sha1,
            343 => Self::Shell,
            344 => Self::Shutdown,
            345 => Self::Sign,
            346 => Self::Sin,
            347 => Self::Sleep,
            348 => Self::SorttextEx,
            349 => Self::Sorttext,
            350 => Self::Sound,
            351 => Self::SpantextChar,
            352 => Self::Spantext,
            353 => Self::SplicetextChar,
            354 => Self::Splicetext,
            355 => Self::SplittextChar,
            356 => Self::Splittext,
            357 => Self::Sqrt,
            358 => Self::Startup,
            359 => Self::Stat,
            360 => Self::Statpanel,
            361 => Self::StepAway,
            362 => Self::StepRand,
            363 => Self::Step,
            364 => Self::StepTo,
            365 => Self::StepTowards,
            366 => Self::Tan,
            367 => Self::Text2asciiChar,
            368 => Self::Text2ascii,
            369 => Self::Text2file,
            370 => Self::Text2num,
            371 => Self::Text2path,
            372 => Self::Text,
            373 => Self::Time2text,
            374 => Self::Trimtext,
            375 => Self::Trunc,
            376 => Self::Turn,
            377 => Self::Typesof,
            378 => Self::Uppertext,
            379 => Self::UrlDecode,
            380 => Self::UrlEncode,
            381 => Self::ValuesCutOver,
            382 => Self::ValuesCutUnder,
            383 => Self::ValuesDot,
            384 => Self::ValuesProduct,
            385 => Self::ValuesSum,
            386 => Self::Vector,
            387 => Self::View,
            388 => Self::Viewers,
            389 => Self::WalkAway,
            390 => Self::WalkRand,
            391 => Self::Walk,
            392 => Self::WalkTo,
            393 => Self::WalkTowards,
            394 => Self::Winclone,
            395 => Self::Winexists,
            396 => Self::Winget,
            397 => Self::Winset,
            398 => Self::Winshow,

            1000 => Self::DatumNew,
            1001 => Self::DatumDel,
            1002 => Self::DatumTopic,
            1003 => Self::DatumRead,
            1004 => Self::DatumWrite,
            1010 => Self::AtomClick,
            1011 => Self::AtomDblClick,
            1012 => Self::AtomMouseDown,
            1013 => Self::AtomMouseDrag,
            1014 => Self::AtomMouseDrop,
            1015 => Self::AtomMouseEntered,
            1016 => Self::AtomMouseExited,
            1017 => Self::AtomMouseMove,
            1018 => Self::AtomMouseUp,
            1019 => Self::AtomMouseWheel,
            1020 => Self::AtomEntered,
            1021 => Self::AtomExited,
            1022 => Self::AtomCrossed,
            1023 => Self::AtomUncrossed,
            1024 => Self::AtomStat,
            1025 => Self::ImageNew,
            1030 => Self::MovableBump,
            1031 => Self::MovableMove,
            1040 => Self::MobLogin,
            1041 => Self::MobLogout,
            1050 => Self::ClientNew,
            1051 => Self::ClientDel,
            1052 => Self::ClientTopic,
            1053 => Self::ClientStat,
            1054 => Self::ClientCommand,
            1055 => Self::ClientImport,
            1056 => Self::ClientExport,
            1057 => Self::ClientAllowUpload,
            1058 => Self::ClientSoundQuery,
            1059 => Self::ClientMeasureText,
            1060 => Self::ClientMove,
            1061 => Self::ClientClick,
            1062 => Self::ClientDblClick,
            1063 => Self::ClientMouseDown,
            1064 => Self::ClientMouseDrag,
            1065 => Self::ClientMouseDrop,
            1066 => Self::ClientMouseEntered,
            1067 => Self::ClientMouseExited,
            1068 => Self::ClientMouseMove,
            1069 => Self::ClientMouseUp,
            1070 => Self::ClientMouseWheel,
            1071 => Self::ClientIsByondMember,
            1072 => Self::ClientCheckPassport,
            1073 => Self::ClientSendPage,
            1074 => Self::ClientGetAPI,
            1075 => Self::ClientSetAPI,
            1076 => Self::ClientRenderIcon,
            1080 => Self::SavefileNew,
            1081 => Self::SavefileFlush,
            1082 => Self::SavefileExportText,
            1083 => Self::SavefileImportText,
            1084 => Self::SavefileLock,
            1085 => Self::SavefileUnlock,
            1090 => Self::VectorNew,
            1091 => Self::VectorCross,
            1092 => Self::VectorDot,
            1093 => Self::VectorInterpolate,
            1094 => Self::VectorNormalize,
            1095 => Self::VectorTurn,
            1096 => Self::PixlocNew,
            1100 => Self::ListAdd,
            1101 => Self::ListCopy,
            1102 => Self::ListCut,
            1103 => Self::ListFind,
            1104 => Self::ListInsert,
            1105 => Self::ListJoin,
            1106 => Self::ListRemove,
            1107 => Self::ListRemoveAll,
            1108 => Self::ListSwap,
            1109 => Self::ListSplice,
            600 => Self::DmNewIcon,
            601 => Self::DmTurnIcon,
            602 => Self::DmFlipIcon,
            603 => Self::DmShiftIcon,
            604 => Self::DmIconIntensity,
            605 => Self::DmIconBlend,
            606 => Self::DmIconSwapColor,
            607 => Self::DmIconDrawBox,
            608 => Self::DmIconInsert,
            609 => Self::DmIconMapColors,
            610 => Self::DmIconScale,
            611 => Self::DmIconCrop,
            612 => Self::DmIconGetPixel,
            613 => Self::DmIconSize,
            620 => Self::DmDatabase,
            630 => Self::MakeGenerator,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::WorldNew => "world.New",
            Self::WorldDel => "world.Del",
            Self::WorldTopic => "world.Topic",
            Self::WorldError => "world.Error",
            Self::WorldReboot => "world.Reboot",
            Self::WorldRepop => "world.Repop",
            Self::WorldExport => "world.Export",
            Self::WorldImport => "world.Import",
            Self::WorldOpenPort => "world.OpenPort",
            Self::WorldProfile => "world.Profile",
            Self::WorldGetConfig => "world.GetConfig",
            Self::WorldSetConfig => "world.SetConfig",
            Self::WorldIsBanned => "world.IsBanned",
            Self::WorldIsSubscribed => "world.IsSubscribed",
            Self::WorldAddCredits => "world.AddCredits",
            Self::WorldGetCredits => "world.GetCredits",
            Self::WorldPayCredits => "world.PayCredits",
            Self::WorldGetScores => "world.GetScores",
            Self::WorldSetScores => "world.SetScores",
            Self::WorldGetMedal => "world.GetMedal",
            Self::WorldSetMedal => "world.SetMedal",
            Self::WorldClearMedal => "world.ClearMedal",
            Self::WorldFile2List => "world.file2list",
            Self::Abs => "abs",
            Self::Addtext => "addtext",
            Self::Alert => "alert",
            Self::Alist => "alist",
            Self::Animate => "animate",
            Self::Arccos => "arccos",
            Self::Arcsin => "arcsin",
            Self::Arctan => "arctan",
            Self::Ascii2text => "ascii2text",
            Self::Astype => "astype",
            Self::Block => "block",
            Self::BoundPixloc => "bound_pixloc",
            Self::BoundsDist => "bounds_dist",
            Self::Bounds => "bounds",
            Self::Browse => "browse",
            Self::BrowseRsc => "browse_rsc",
            Self::Ceil => "ceil",
            Self::CkeyEx => "ckeyEx",
            Self::Ckey => "ckey",
            Self::Clamp => "clamp",
            Self::CmptextEx => "cmptextEx",
            Self::Cmptext => "cmptext",
            Self::CopytextChar => "copytext_char",
            Self::Copytext => "copytext",
            Self::Cos => "cos",
            Self::Crash => "CRASH",
            Self::DmDbClose => "_dm_db_close",
            Self::DmDbColumns => "_dm_db_columns",
            Self::DmDbConnect => "_dm_db_connect",
            Self::DmDbErrorMsg => "_dm_db_error_msg",
            Self::DmDbExecute => "_dm_db_execute",
            Self::DmDbIsConnected => "_dm_db_is_connected",
            Self::DmDbNewCon => "_dm_db_new_con",
            Self::DmDbNewQuery => "_dm_db_new_query",
            Self::DmDbNextRow => "_dm_db_next_row",
            Self::DmDbQuote => "_dm_db_quote",
            Self::DmDbRowCount => "_dm_db_row_count",
            Self::DmDbRowsAffected => "_dm_db_rows_affected",
            Self::FcopyRsc => "fcopy_rsc",
            Self::Fcopy => "fcopy",
            Self::Fdel => "fdel",
            Self::Fexists => "fexists",
            Self::File2text => "file2text",
            Self::File => "file",
            Self::Filter => "filter",
            Self::FindlasttextChar => "findlasttext_char",
            Self::FindlasttextExChar => "findlasttextEx_char",
            Self::FindlasttextEx => "findlasttextEx",
            Self::Findlasttext => "findlasttext",
            Self::FindtextChar => "findtext_char",
            Self::FindtextExChar => "findtextEx_char",
            Self::FindtextEx => "findtextEx",
            Self::Findtext => "findtext",
            Self::Flick => "flick",
            Self::Flist => "flist",
            Self::Floor => "floor",
            Self::Fract => "fract",
            Self::Ftime => "ftime",
            Self::Ftp => "ftp",
            Self::GetDir => "get_dir",
            Self::GetDist => "get_dist",
            Self::GetStepAway => "get_step_away",
            Self::GetStepRand => "get_step_rand",
            Self::GetStep => "get_step",
            Self::GetStepsTo => "get_steps_to",
            Self::GetStepTo => "get_step_to",
            Self::GetStepTowards => "get_step_towards",
            Self::Generator => "generator",
            Self::Gradient => "gradient",
            Self::Hascall => "hascall",
            Self::Hearers => "hearers",
            Self::HtmlDecode => "html_decode",
            Self::HtmlEncode => "html_encode",
            Self::Icon => "icon",
            Self::IconStates => "icon_states",
            Self::Image => "image",
            Self::Isarea => "isarea",
            Self::Isfile => "isfile",
            Self::Isicon => "isicon",
            Self::Isinf => "isinf",
            Self::Islist => "islist",
            Self::Isloc => "isloc",
            Self::Ismob => "ismob",
            Self::Ismovable => "ismovable",
            Self::Isnan => "isnan",
            Self::Isnull => "isnull",
            Self::Isnum => "isnum",
            Self::Isobj => "isobj",
            Self::Ispath => "ispath",
            Self::Ispointer => "ispointer",
            Self::Issaved => "issaved",
            Self::Istext => "istext",
            Self::Isturf => "isturf",
            Self::Istype => "istype",
            Self::Jointext => "jointext",
            Self::JsonDecode => "json_decode",
            Self::JsonEncode => "json_encode",
            Self::LengthChar => "length_char",
            Self::Length => "length",
            Self::Lentext => "lentext",
            Self::Lerp => "lerp",
            Self::Link => "link",
            Self::List2params => "list2params",
            Self::LoadExt => "load_ext",
            Self::LoadResource => "load_resource",
            Self::Locate => "locate",
            Self::Log => "log",
            Self::Lowertext => "lowertext",
            Self::Matrix => "matrix",
            Self::Max => "max",
            Self::Md5 => "md5",
            Self::Min => "min",
            Self::Missile => "missile",
            Self::MutableAppearance => "mutable_appearance",
            Self::Newlist => "newlist",
            Self::NoiseHash => "noise_hash",
            Self::NonspantextChar => "nonspantext_char",
            Self::Nameof => "nameof",
            Self::Nonspantext => "nonspantext",
            Self::Num2text => "num2text",
            Self::Obounds => "obounds",
            Self::Ohearers => "ohearers",
            Self::Orange => "orange",
            Self::Output => "output",
            Self::Oview => "oview",
            Self::Oviewers => "oviewers",
            Self::Params2list => "params2list",
            Self::Pixloc => "pixloc",
            Self::Prob => "prob",
            Self::Rand => "rand",
            Self::RandSeed => "rand_seed",
            Self::Range => "range",
            Self::Ref => "ref",
            Self::Refcount => "refcount",
            Self::Regex => "regex",
            Self::ReplacetextChar => "replacetext_char",
            Self::ReplacetextExChar => "replacetextEx_char",
            Self::ReplacetextEx => "replacetextEx",
            Self::Replacetext => "replacetext",
            Self::Rgb2num => "rgb2num",
            Self::Rgb => "rgb",
            Self::Roll => "roll",
            Self::Round => "round",
            Self::Run => "run",
            Self::Sha1 => "sha1",
            Self::Shell => "shell",
            Self::Shutdown => "shutdown",
            Self::Sign => "sign",
            Self::Sin => "sin",
            Self::Sleep => "sleep",
            Self::SorttextEx => "sorttextEx",
            Self::Sorttext => "sorttext",
            Self::Sound => "sound",
            Self::SpantextChar => "spantext_char",
            Self::Spantext => "spantext",
            Self::SplicetextChar => "splicetext_char",
            Self::Splicetext => "splicetext",
            Self::SplittextChar => "splittext_char",
            Self::Splittext => "splittext",
            Self::Sqrt => "sqrt",
            Self::Startup => "startup",
            Self::Stat => "stat",
            Self::Statpanel => "statpanel",
            Self::StepAway => "step_away",
            Self::StepRand => "step_rand",
            Self::Step => "step",
            Self::StepTo => "step_to",
            Self::StepTowards => "step_towards",
            Self::Tan => "tan",
            Self::Text2asciiChar => "text2ascii_char",
            Self::Text2ascii => "text2ascii",
            Self::Text2file => "text2file",
            Self::Text2num => "text2num",
            Self::Text2path => "text2path",
            Self::Text => "text",
            Self::Time2text => "time2text",
            Self::Trimtext => "trimtext",
            Self::Trunc => "trunc",
            Self::Turn => "turn",
            Self::Typesof => "typesof",
            Self::Uppertext => "uppertext",
            Self::UrlDecode => "url_decode",
            Self::UrlEncode => "url_encode",
            Self::ValuesCutOver => "values_cut_over",
            Self::ValuesCutUnder => "values_cut_under",
            Self::ValuesDot => "values_dot",
            Self::ValuesProduct => "values_product",
            Self::ValuesSum => "values_sum",
            Self::Vector => "vector",
            Self::View => "view",
            Self::Viewers => "viewers",
            Self::WalkAway => "walk_away",
            Self::WalkRand => "walk_rand",
            Self::Walk => "walk",
            Self::WalkTo => "walk_to",
            Self::WalkTowards => "walk_towards",
            Self::Winclone => "winclone",
            Self::Winexists => "winexists",
            Self::Winget => "winget",
            Self::Winset => "winset",
            Self::Winshow => "winshow",

            Self::DatumNew => "datum.New",
            Self::DatumDel => "datum.Del",
            Self::DatumTopic => "datum.Topic",
            Self::DatumRead => "datum.Read",
            Self::DatumWrite => "datum.Write",
            Self::AtomClick => "atom.Click",
            Self::AtomDblClick => "atom.DblClick",
            Self::AtomMouseDown => "atom.MouseDown",
            Self::AtomMouseDrag => "atom.MouseDrag",
            Self::AtomMouseDrop => "atom.MouseDrop",
            Self::AtomMouseEntered => "atom.MouseEntered",
            Self::AtomMouseExited => "atom.MouseExited",
            Self::AtomMouseMove => "atom.MouseMove",
            Self::AtomMouseUp => "atom.MouseUp",
            Self::AtomMouseWheel => "atom.MouseWheel",
            Self::AtomEntered => "atom.Entered",
            Self::AtomExited => "atom.Exited",
            Self::AtomCrossed => "atom.Crossed",
            Self::AtomUncrossed => "atom.Uncrossed",
            Self::AtomStat => "atom.Stat",
            Self::ImageNew => "image.New",
            Self::MovableBump => "movable.Bump",
            Self::MovableMove => "movable.Move",
            Self::MobLogin => "mob.Login",
            Self::MobLogout => "mob.Logout",
            Self::ClientNew => "client.New",
            Self::ClientDel => "client.Del",
            Self::ClientTopic => "client.Topic",
            Self::ClientStat => "client.Stat",
            Self::ClientCommand => "client.Command",
            Self::ClientImport => "client.Import",
            Self::ClientExport => "client.Export",
            Self::ClientAllowUpload => "client.AllowUpload",
            Self::ClientSoundQuery => "client.SoundQuery",
            Self::ClientMeasureText => "client.MeasureText",
            Self::ClientMove => "client.Move",
            Self::ClientClick => "client.Click",
            Self::ClientDblClick => "client.DblClick",
            Self::ClientMouseDown => "client.MouseDown",
            Self::ClientMouseDrag => "client.MouseDrag",
            Self::ClientMouseDrop => "client.MouseDrop",
            Self::ClientMouseEntered => "client.MouseEntered",
            Self::ClientMouseExited => "client.MouseExited",
            Self::ClientMouseMove => "client.MouseMove",
            Self::ClientMouseUp => "client.MouseUp",
            Self::ClientMouseWheel => "client.MouseWheel",
            Self::ClientIsByondMember => "client.IsByondMember",
            Self::ClientCheckPassport => "client.CheckPassport",
            Self::ClientSendPage => "client.SendPage",
            Self::ClientGetAPI => "client.GetAPI",
            Self::ClientSetAPI => "client.SetAPI",
            Self::ClientRenderIcon => "client.RenderIcon",
            Self::SavefileNew => "savefile.New",
            Self::SavefileFlush => "savefile.Flush",
            Self::SavefileExportText => "savefile.ExportText",
            Self::SavefileImportText => "savefile.ImportText",
            Self::SavefileLock => "savefile.Lock",
            Self::SavefileUnlock => "savefile.Unlock",
            Self::VectorNew => "vector.New",
            Self::VectorCross => "vector.Cross",
            Self::VectorDot => "vector.Dot",
            Self::VectorInterpolate => "vector.Interpolate",
            Self::VectorNormalize => "vector.Normalize",
            Self::VectorTurn => "vector.Turn",
            Self::PixlocNew => "pixloc.New",
            Self::ListAdd => "list.Add",
            Self::ListCopy => "list.Copy",
            Self::ListCut => "list.Cut",
            Self::ListFind => "list.Find",
            Self::ListInsert => "list.Insert",
            Self::ListJoin => "list.Join",
            Self::ListRemove => "list.Remove",
            Self::ListRemoveAll => "list.RemoveAll",
            Self::ListSwap => "list.Swap",
            Self::ListSplice => "list.Splice",
            Self::DmNewIcon => "_dm_new_icon",
            Self::DmTurnIcon => "_dm_turn_icon",
            Self::DmFlipIcon => "_dm_flip_icon",
            Self::DmShiftIcon => "_dm_shift_icon",
            Self::DmIconIntensity => "_dm_icon_intensity",
            Self::DmIconBlend => "_dm_icon_blend",
            Self::DmIconSwapColor => "_dm_icon_swap_color",
            Self::DmIconDrawBox => "_dm_icon_draw_box",
            Self::DmIconInsert => "_dm_icon_insert",
            Self::DmIconMapColors => "_dm_icon_map_colors",
            Self::DmIconScale => "_dm_icon_scale",
            Self::DmIconCrop => "_dm_icon_crop",
            Self::DmIconGetPixel => "_dm_icon_getpixel",
            Self::DmIconSize => "_dm_icon_size",
            Self::DmDatabase => "_dm_database",
            Self::MakeGenerator => "make_generator",
        }
    }
}

impl Evaluator<'_> {
    pub(crate) fn intrinsic(
        &mut self, id: u32, receiver: Receiver, params: &[Identifier], args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let Some(intrinsic) = Intrinsic::from_id(id) else {
            return Err(self.fault(FaultKind::Unsupported(format!("intrinsic {id}"))));
        };

        self.intrinsic_impl(intrinsic, receiver, params, args)
    }

    pub(crate) fn intrinsic_impl(
        &mut self, intrinsic: Intrinsic, receiver: Receiver, params: &[Identifier],
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|(_, value)| value.text()).unwrap_or_default();
        let positional = || args.iter().map(|(_, value)| value.clone()).collect::<Vec<_>>();

        match intrinsic {
            Intrinsic::WorldNew | Intrinsic::WorldDel | Intrinsic::WorldTopic | Intrinsic::WorldError => {
                Ok(GenericValue::Null)
            },

            Intrinsic::WorldIsSubscribed
            | Intrinsic::WorldGetScores
            | Intrinsic::WorldGetMedal
            | Intrinsic::WorldGetCredits => Ok(GenericValue::Null),
            Intrinsic::WorldIsBanned => Ok(false.into()),
            Intrinsic::WorldAddCredits | Intrinsic::WorldPayCredits => Ok(0.into()),
            Intrinsic::WorldSetScores | Intrinsic::WorldSetMedal | Intrinsic::WorldClearMedal => Ok(GenericValue::Null),
            Intrinsic::WorldProfile | Intrinsic::WorldGetConfig | Intrinsic::WorldSetConfig => Ok(GenericValue::Null),

            Intrinsic::WorldReboot
            | Intrinsic::WorldRepop
            | Intrinsic::WorldExport
            | Intrinsic::WorldImport
            | Intrinsic::WorldOpenPort => Err(self.fault(FaultKind::Blocked(intrinsic.name().into()))),

            Intrinsic::WorldFile2List => self.file2list(&arg(0), &arg(1)),

            Intrinsic::DatumNew
            | Intrinsic::DatumDel
            | Intrinsic::DatumTopic
            | Intrinsic::AtomClick
            | Intrinsic::AtomDblClick
            | Intrinsic::AtomMouseDown
            | Intrinsic::AtomMouseDrag
            | Intrinsic::AtomMouseDrop
            | Intrinsic::AtomMouseEntered
            | Intrinsic::AtomMouseExited
            | Intrinsic::AtomMouseMove
            | Intrinsic::AtomMouseUp
            | Intrinsic::AtomMouseWheel
            | Intrinsic::AtomEntered
            | Intrinsic::AtomExited
            | Intrinsic::AtomCrossed
            | Intrinsic::AtomUncrossed
            | Intrinsic::AtomStat
            | Intrinsic::MovableBump => Ok(GenericValue::Null),

            Intrinsic::DmNewIcon => Ok(arg(0)),

            Intrinsic::DatumRead
            | Intrinsic::DatumWrite
            | Intrinsic::MovableMove
            | Intrinsic::MobLogin
            | Intrinsic::MobLogout
            | Intrinsic::ClientNew
            | Intrinsic::ClientDel
            | Intrinsic::ClientTopic
            | Intrinsic::ClientStat
            | Intrinsic::ClientCommand
            | Intrinsic::ClientImport
            | Intrinsic::ClientExport
            | Intrinsic::ClientAllowUpload
            | Intrinsic::ClientSoundQuery
            | Intrinsic::ClientMeasureText
            | Intrinsic::ClientMove
            | Intrinsic::ClientClick
            | Intrinsic::ClientDblClick
            | Intrinsic::ClientMouseDown
            | Intrinsic::ClientMouseDrag
            | Intrinsic::ClientMouseDrop
            | Intrinsic::ClientMouseEntered
            | Intrinsic::ClientMouseExited
            | Intrinsic::ClientMouseMove
            | Intrinsic::ClientMouseUp
            | Intrinsic::ClientMouseWheel
            | Intrinsic::ClientIsByondMember
            | Intrinsic::ClientCheckPassport
            | Intrinsic::ClientSendPage
            | Intrinsic::ClientGetAPI
            | Intrinsic::ClientSetAPI
            | Intrinsic::ClientRenderIcon
            | Intrinsic::SavefileNew
            | Intrinsic::SavefileFlush
            | Intrinsic::SavefileExportText
            | Intrinsic::SavefileImportText
            | Intrinsic::SavefileLock
            | Intrinsic::SavefileUnlock
            | Intrinsic::VectorNew
            | Intrinsic::VectorCross
            | Intrinsic::VectorDot
            | Intrinsic::VectorInterpolate
            | Intrinsic::VectorNormalize
            | Intrinsic::VectorTurn
            | Intrinsic::PixlocNew
            | Intrinsic::DmTurnIcon
            | Intrinsic::DmFlipIcon
            | Intrinsic::DmShiftIcon
            | Intrinsic::DmIconIntensity
            | Intrinsic::DmIconBlend
            | Intrinsic::DmIconSwapColor
            | Intrinsic::DmIconDrawBox
            | Intrinsic::DmIconInsert
            | Intrinsic::DmIconMapColors
            | Intrinsic::DmIconScale
            | Intrinsic::DmIconCrop
            | Intrinsic::DmIconGetPixel
            | Intrinsic::DmIconSize
            | Intrinsic::DmDatabase
            | Intrinsic::MakeGenerator => Err(self.fault(FaultKind::Blocked(intrinsic.name().into()))),

            Intrinsic::Animate
            | Intrinsic::Browse
            | Intrinsic::BrowseRsc
            | Intrinsic::Fcopy
            | Intrinsic::Fdel
            | Intrinsic::Flick
            | Intrinsic::Ftp
            | Intrinsic::Link
            | Intrinsic::Shell
            | Intrinsic::Shutdown
            | Intrinsic::Sleep
            | Intrinsic::Text2file
            | Intrinsic::Winget
            | Intrinsic::Winset => Err(self.fault(FaultKind::Blocked(intrinsic.name().into()))),

            Intrinsic::Image | Intrinsic::MutableAppearance => {
                self.appearance_object(intrinsic.name(), None, params, args)
            },
            Intrinsic::ImageNew => self.appearance_object("image", receiver.object(), params, args),
            Intrinsic::Issaved => Ok(true.into()),

            Intrinsic::FcopyRsc => Ok(arg(0)),

            Intrinsic::ListAdd
            | Intrinsic::ListCopy
            | Intrinsic::ListCut
            | Intrinsic::ListFind
            | Intrinsic::ListInsert
            | Intrinsic::ListJoin
            | Intrinsic::ListRemove
            | Intrinsic::ListRemoveAll
            | Intrinsic::ListSwap
            | Intrinsic::ListSplice => {
                let id = receiver.list().ok_or_else(|| self.fault(FaultKind::InvalidReference))?;

                self.list_builtin(intrinsic, id, args.into_iter().map(|(_, value)| value).collect())
            },

            Intrinsic::Nameof => match arg(0) {
                GenericValue::Path(path) => match path.name() {
                    Some(name) => self.text(name.to_string()),
                    None => Ok(GenericValue::Null),
                },
                _ => Err(self.fault(FaultKind::Blocked("nameof of a non-path".into()))),
            },

            Intrinsic::File2text => match self.resource_path(&arg(0))? {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) if text.len() > self.limits.text_bytes => Err(self.fault(FaultKind::Memory)),
                    Ok(text) => self.text(text),
                    Err(_) => Ok(GenericValue::Null),
                },
                None => Ok(GenericValue::Null),
            },
            Intrinsic::Fexists => Ok(self.resource_path(&arg(0))?.is_some_and(|path| path.is_file()).into()),
            Intrinsic::File => match self.resource_path(&arg(0))? {
                Some(_) => Ok(GenericValue::Resource(text(0).into())),
                None => Ok(GenericValue::Null),
            },
            Intrinsic::Flist => {
                let Some(path) = self.resource_path(&arg(0))? else {
                    return Ok(GenericValue::Null);
                };
                let Ok(entries) = std::fs::read_dir(&path) else {
                    return self.list(Vec::new());
                };

                let mut names = entries
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        let name = entry.file_name().into_string().ok()?;

                        Some(match entry.path().is_dir() {
                            true => format!("{name}/"),
                            false => name,
                        })
                    })
                    .collect::<Vec<_>>();
                names.sort();
                self.reserve(names.len())?;

                let entries = names.into_iter().map(|name| (GenericValue::from(name), None)).collect();
                self.list(entries)
            },

            Intrinsic::Isnull => Ok(matches!(arg(0), GenericValue::Null).into()),
            Intrinsic::Isnum => Ok(matches!(arg(0), GenericValue::Num(_)).into()),
            Intrinsic::Istext => Ok(matches!(arg(0), GenericValue::Text(_)).into()),
            Intrinsic::Islist => Ok(matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)).into()),
            Intrinsic::Isicon => Ok(match arg(0) {
                GenericValue::Resource(path) => path.rsplit_once('.').is_some_and(|(_, extension)| {
                    ["dmi", "bmp", "png", "jpg", "gif"]
                        .iter()
                        .any(|format| extension.eq_ignore_ascii_case(format))
                }),
                GenericValue::Object(id) => self
                    .runtime
                    .heap
                    .object(id)
                    .zip(self.tree.id_of(&TreePath::parse("/icon")))
                    .is_some_and(|(object, icon)| self.tree.is_subtype_of(object.ty, icon)),
                _ => false,
            }
            .into()),
            Intrinsic::Ispath | Intrinsic::Istype => {
                let ty = match arg(0) {
                    GenericValue::Object(id) if intrinsic == Intrinsic::Istype => {
                        self.runtime.heap.object(id).map(|o| o.ty)
                    },
                    GenericValue::Path(path) => self.tree.id_of(&path),
                    _ => None,
                };
                let base = match arg(1) {
                    GenericValue::Path(path) => self.tree.id_of(&path),
                    _ => None,
                };
                Ok(ty
                    .is_some_and(|ty| base.is_none_or(|base| self.tree.is_subtype_of(ty, base)))
                    .into())
            },
            Intrinsic::Isturf
            | Intrinsic::Isarea
            | Intrinsic::Ismovable
            | Intrinsic::Isobj
            | Intrinsic::Ismob
            | Intrinsic::Isloc => {
                let roots = self.tree.roots();
                let root = match intrinsic {
                    Intrinsic::Isturf => roots.turf,
                    Intrinsic::Isarea => roots.area,
                    Intrinsic::Ismovable => roots.movable,
                    Intrinsic::Isobj => roots.obj,
                    Intrinsic::Ismob => roots.mob,
                    _ => roots.atom,
                };

                Ok(self.is_root_type(&arg(0), root).into())
            },
            Intrinsic::Length | Intrinsic::LengthChar => Ok((match arg(0) {
                GenericValue::List(id) | GenericValue::ArgList(id) => {
                    self.runtime.heap.list(id).map_or(0, |l| l.entries.len())
                },
                GenericValue::Text(s) | GenericValue::Resource(s) => s.chars().count(),
                GenericValue::Null => 0,
                GenericValue::Object(id) => self.runtime.heap.object(id).map_or(0, |o| o.contents.len()),
                _ => 0,
            } as f32)
                .into()),
            Intrinsic::Typesof => self.types_of(positional(), true),
            Intrinsic::Text2path => {
                let path = TreePath::parse(text(0));
                Ok(self
                    .tree
                    .id_of(&path)
                    .map(|_| GenericValue::Path(path))
                    .unwrap_or_default())
            },
            Intrinsic::Min | Intrinsic::Max => {
                let args = positional();
                let values = if args.len() == 1 && matches!(arg(0), GenericValue::List(_) | GenericValue::ArgList(_)) {
                    self.iter_values(arg(0))?.into_iter().map(|(v, _)| v).collect()
                } else {
                    args
                };
                let mut result: Option<f32> = None;
                for value in values {
                    let n = self.number(&value)?;
                    result = Some(result.map_or(n, |r| {
                        if intrinsic == Intrinsic::Min {
                            r.min(n)
                        } else {
                            r.max(n)
                        }
                    }));
                }
                Ok(result.unwrap_or(0.0).into())
            },
            Intrinsic::Abs => Ok(self.number(&arg(0))?.abs().into()),
            Intrinsic::Ceil => Ok(self.number(&arg(0))?.ceil().into()),
            Intrinsic::Floor => Ok(self.number(&arg(0))?.floor().into()),
            Intrinsic::Sqrt => Ok(self.number(&arg(0))?.sqrt().into()),
            Intrinsic::Round => {
                let n = self.number(&arg(0))?;
                if args.len() < 2 {
                    Ok(n.floor().into())
                } else {
                    let step = self.number(&arg(1))?;
                    Ok(if step == 0.0 {
                        n
                    } else {
                        (n / step + 0.5).floor() * step
                    }
                    .into())
                }
            },
            Intrinsic::Clamp => {
                let n = self.number(&arg(0))?;
                let lo = self.number(&arg(1))?;
                let hi = self.number(&arg(2))?;
                if lo.is_nan() || hi.is_nan() || lo > hi {
                    return Err(self.fault(FaultKind::InvalidOperation("invalid clamp bounds".into())));
                }

                Ok(n.clamp(lo, hi).into())
            },
            Intrinsic::Rand => {
                let (lo, hi) = match args.len() {
                    0 => (0.0, 1.0),
                    1 => (0.0, self.number(&arg(0))?),
                    _ => (self.number(&arg(0))?, self.number(&arg(1))?),
                };
                let empty = args.is_empty();
                let r = self.random();

                Ok(if empty {
                    r
                } else {
                    lo.min(hi) + (r * ((hi - lo).abs() + 1.0)).floor()
                }
                .into())
            },
            Intrinsic::Prob => {
                let n = self.number(&arg(0))?;

                Ok((self.random() * 100.0 < n).into())
            },
            Intrinsic::GetStep => {
                let dir = self.number(&arg(1))? as i32;
                let query = arg(0)
                    .object()
                    .and_then(|id| self.runtime.world.position(&self.runtime.heap, id))
                    .map(|p| p.step(dir));
                let origin = self
                    .origin
                    .and_then(|id| self.runtime.world.position(&self.runtime.heap, id));
                if let (Some(a), Some(b)) = (origin, query)
                    && (a.z != b.z || a.x.abs_diff(b.x) > 1 || a.y.abs_diff(b.y) > 1)
                {
                    self.memo_safe = false;
                }

                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.world.get_step(&self.runtime.heap, id, dir))
                    .map(GenericValue::Object)
                    .unwrap_or_default())
            },
            Intrinsic::GetDir | Intrinsic::GetDist => {
                self.position_sensitive = true;
                let pos = |value: GenericValue, evaluator: &Self| {
                    value
                        .object()
                        .and_then(|id| evaluator.runtime.world.position(&evaluator.runtime.heap, id))
                };
                let (Some(a), Some(b)) = (pos(arg(0), self), pos(arg(1), self)) else {
                    return Ok(0.0.into());
                };

                if intrinsic == Intrinsic::GetDir {
                    Ok(((i32::from(b.x > a.x) * 4)
                        | (i32::from(b.x < a.x) * 8)
                        | i32::from(b.y > a.y)
                        | (i32::from(b.y < a.y) * 2))
                        .into())
                } else {
                    Ok(if a.z != b.z {
                        -1.0
                    } else {
                        a.x.abs_diff(b.x).max(a.y.abs_diff(b.y)) as f32
                    }
                    .into())
                }
            },
            Intrinsic::Locate => {
                if args.len() >= 3 || args.len() < 2 {
                    self.memo_safe = false;
                }

                if args.len() >= 3 {
                    let pos = Position::new(
                        self.number(&arg(0))? as i32,
                        self.number(&arg(1))? as i32,
                        self.number(&arg(2))? as i32,
                    );

                    return Ok(self
                        .runtime
                        .world
                        .turf_at(pos)
                        .map(GenericValue::Object)
                        .unwrap_or_default());
                }

                let values = self.iter_values(args.get(1).map(|(_, v)| v.clone()).unwrap_or(GenericValue::World))?;
                let query = arg(0);
                let base = if let GenericValue::Path(path) = &query {
                    self.tree.id_of(path)
                } else {
                    None
                };

                Ok(values
                    .into_iter()
                    .find(|(v, _)| {
                        base.map_or(*v == query, |base| {
                            v.object()
                                .and_then(|id| self.runtime.heap.object(id))
                                .is_some_and(|o| self.tree.is_subtype_of(o.ty, base))
                        })
                    })
                    .map(|(v, _)| v)
                    .unwrap_or_default())
            },
            Intrinsic::Turn => {
                let dirs = [1, 9, 8, 10, 2, 6, 4, 5];
                let dir = self.number(&arg(0))? as i32;
                let angle = self.number(&arg(1))?;
                let Some(index) = dirs.iter().position(|d| *d == dir) else {
                    return Ok(dir.into());
                };
                let index = (index as i32 + (angle.rem_euclid(360.0) / 45.0).round() as i32).rem_euclid(8) as usize;

                Ok(dirs.get(index).copied().unwrap_or(0).into())
            },
            Intrinsic::Rgb
                if (3..=4).contains(&args.len()) && args.iter().all(|(_, a)| matches!(a, GenericValue::Num(_))) =>
            {
                let mut color = String::from("#");
                for index in 0..args.len() {
                    let channel = self.number(&arg(index))?.round().clamp(0.0, 255.0) as u8;
                    color.push_str(&format!("{channel:02x}"));
                }

                self.text(color)
            },
            Intrinsic::Rgb => Err(self.fault(FaultKind::Blocked(intrinsic.name().into()))),
            Intrinsic::Num2text => {
                let n = self.number(&arg(0))?;

                self.text(n.to_string())
            },
            Intrinsic::Text2num => Ok(text(0).trim().parse::<f32>().map(GenericValue::Num).unwrap_or_default()),
            Intrinsic::Uppertext => {
                let t = text(0).to_uppercase();

                self.text(t)
            },
            Intrinsic::Lowertext => {
                let t = text(0).to_lowercase();

                self.text(t)
            },
            Intrinsic::Sorttext | Intrinsic::SorttextEx => {
                self.charge(text(0).len().saturating_add(text(1).len()))?;
                let order = if intrinsic == Intrinsic::SorttextEx {
                    text(1).cmp(text(0))
                } else {
                    text(1).to_lowercase().cmp(&text(0).to_lowercase())
                };

                Ok((order as i32).into())
            },
            Intrinsic::Copytext | Intrinsic::CopytextChar => {
                let s = text(0);
                let chars = s.chars().collect::<Vec<_>>();
                self.charge(chars.len())?;
                let args = positional();
                let start = index_arg(&args, 1, 1, chars.len() + 1);
                let end = index_arg(&args, 2, 0, chars.len() + 1);

                self.text(chars.get(start.min(end)..end).unwrap_or_default().iter().collect())
            },
            Intrinsic::Findtext | Intrinsic::FindtextEx | Intrinsic::Findlasttext | Intrinsic::FindlasttextEx => {
                if matches!(arg(1), GenericValue::Object(_)) {
                    return Err(self.fault(FaultKind::Blocked("regex matching".into())));
                }

                let case = matches!(intrinsic, Intrinsic::FindtextEx | Intrinsic::FindlasttextEx);
                let last = matches!(intrinsic, Intrinsic::Findlasttext | Intrinsic::FindlasttextEx);
                let hay = if case {
                    text(0).to_string()
                } else {
                    text(0).to_lowercase()
                };
                let needle = if case {
                    text(1).to_string()
                } else {
                    text(1).to_lowercase()
                };
                self.charge(hay.len().saturating_add(needle.len()))?;
                let chars = hay.chars().collect::<Vec<_>>();
                let args = positional();
                let start = index_arg(&args, 2, 1, chars.len() + 1);
                let end = index_arg(&args, 3, 0, chars.len() + 1);
                let segment: String = chars.get(start.min(end)..end).unwrap_or_default().iter().collect();
                let found = if last {
                    segment.rfind(&needle)
                } else {
                    segment.find(&needle)
                };

                Ok(found
                    .map(|offset| start + segment.get(..offset).unwrap_or_default().chars().count() + 1)
                    .unwrap_or(0)
                    .into())
            },
            Intrinsic::Replacetext | Intrinsic::ReplacetextEx => {
                if matches!(arg(1), GenericValue::Object(_)) {
                    return Err(self.fault(FaultKind::Blocked("regex matching".into())));
                }

                let source = text(0);
                let needle = text(1);
                let replacement = text(2);
                if needle.is_empty() {
                    return self.text(source.to_string());
                }

                let mut output = String::new();
                let mut offset = 0;
                while let Some(rest) = source.get(offset..) {
                    self.charge(1)?;
                    if rest.is_empty() {
                        break;
                    }

                    let candidate = rest.get(..needle.len());
                    let found = candidate.is_some_and(|s| {
                        if intrinsic == Intrinsic::ReplacetextEx {
                            s == needle
                        } else {
                            s.eq_ignore_ascii_case(needle)
                        }
                    });
                    let piece = if found {
                        offset += needle.len();
                        replacement
                    } else {
                        let length = rest.chars().next().map(char::len_utf8).unwrap_or(1);
                        offset += length;
                        rest.get(..length).unwrap_or_default()
                    };
                    if output.len().saturating_add(piece.len()) > self.limits.text_bytes {
                        return Err(self.fault(FaultKind::Memory));
                    }

                    output.push_str(piece);
                }

                self.text(output)
            },
            Intrinsic::Splittext => {
                let source = text(0);
                let delimiter = text(1);
                self.charge(source.len())?;
                let entries = if delimiter.is_empty() {
                    source
                        .chars()
                        .map(|c| (GenericValue::from(c.to_string()), None))
                        .collect()
                } else {
                    source.split(delimiter).map(|s| (GenericValue::from(s), None)).collect()
                };

                self.list(entries)
            },
            Intrinsic::JsonDecode => {
                let value = crate::json::decode(
                    text(0),
                    self.limits.allocations.min(self.instruction_budget_remaining as usize),
                )
                .map_err(|e| self.fault(e))?;
                self.reserve(text(0).len())?;

                self.constant(&value)
            },
            Intrinsic::Ref => {
                self.memo_safe = false;

                self.text(arg(0).display())
            },
            Intrinsic::Hascall => {
                let name = Identifier::from(text(1));

                Ok(arg(0)
                    .object()
                    .and_then(|id| self.runtime.heap.object(id))
                    .and_then(|o| self.find_proc(o.ty, &name))
                    .is_some()
                    .into())
            },
            Intrinsic::Crash => Err(self.fault(FaultKind::InvalidOperation(arg(0).display()))),
            Intrinsic::Icon => self.construct("/icon", params, None, args),
            Intrinsic::Sound => self.construct("/sound", params, None, args),
            Intrinsic::Matrix => {
                let matrix = self.tree.id_of(&TreePath::parse("/matrix"));
                match self.is_root_type(&arg(0), matrix) {
                    true => Ok(arg(0)),
                    false => self.construct("/matrix", params, None, args),
                }
            },
            Intrinsic::Regex => {
                let (target, pattern, flags) = match arg(0) {
                    GenericValue::Object(id) => (Some(id), arg(1), arg(2)),
                    _ => (None, arg(0), arg(1)),
                };
                let id = match target {
                    Some(id) => id,
                    None => {
                        let ty = self
                            .tree
                            .id_of(&TreePath::parse("/regex"))
                            .ok_or_else(|| self.fault(FaultKind::MissingVariable("/regex".into())))?;
                        self.reserve(1)?;

                        self.runtime
                            .heap
                            .alloc_object(crate::heap::Object::new(ty))
                            .map_err(|kind| self.fault(kind))?
                    },
                };

                self.write_field(GenericValue::Object(id), "name".into(), pattern)?;
                self.write_field(GenericValue::Object(id), "flags".into(), flags)?;

                Ok(GenericValue::Object(id))
            },

            Intrinsic::Trimtext => {
                let trimmed = text(0).trim().to_string();

                self.text(trimmed)
            },
            Intrinsic::Ckey | Intrinsic::CkeyEx => {
                let fold = intrinsic == Intrinsic::Ckey;
                let key = text(0)
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .map(|c| if fold { c.to_ascii_lowercase() } else { c })
                    .collect::<String>();

                self.text(key)
            },
            Intrinsic::Cmptext | Intrinsic::CmptextEx => {
                let exact = intrinsic == Intrinsic::CmptextEx;
                let first = text(0);
                self.charge(args.len())?;

                Ok((1..args.len())
                    .all(|index| match exact {
                        true => text(index) == first,
                        false => text(index).eq_ignore_ascii_case(first),
                    })
                    .into())
            },
            Intrinsic::Addtext => {
                let mut output = String::new();
                for (_, value) in &args {
                    let piece = value.display();
                    if output.len().saturating_add(piece.len()) > self.limits.text_bytes {
                        return Err(self.fault(FaultKind::Memory));
                    }

                    output.push_str(&piece);
                }

                self.text(output)
            },
            Intrinsic::Jointext => {
                let entries = self.iter_values(arg(0))?;
                let glue = text(1).to_string();
                let positional = positional();
                let start = index_arg(&positional, 2, 1, entries.len() + 1);
                let end = index_arg(&positional, 3, 0, entries.len() + 1);

                let mut output = String::new();
                for (index, (value, _)) in entries.get(start.min(end)..end).unwrap_or_default().iter().enumerate() {
                    let piece = value.display();
                    let extra = piece.len() + if index == 0 { 0 } else { glue.len() };
                    if output.len().saturating_add(extra) > self.limits.text_bytes {
                        return Err(self.fault(FaultKind::Memory));
                    }

                    if index > 0 {
                        output.push_str(&glue);
                    }
                    output.push_str(&piece);
                }

                self.text(output)
            },
            Intrinsic::Text2ascii | Intrinsic::Text2asciiChar => {
                let position = match args.len() {
                    0 | 1 => 1.0,
                    _ => self.number(&arg(1))?,
                };
                let chars = text(0).chars().collect::<Vec<_>>();
                let index = index_arg(&[GenericValue::Num(position)], 0, 1, chars.len() + 1);

                Ok(chars.get(index).map(|c| *c as u32 as f32).unwrap_or(0.0).into())
            },
            Intrinsic::Ascii2text => {
                let code = self.number(&arg(0))?;
                let Some(character) = u32::try_from(code as i64).ok().and_then(char::from_u32) else {
                    return Ok(GenericValue::Null);
                };

                self.text(character.to_string())
            },

            Intrinsic::Sin => Ok(self.number(&arg(0))?.to_radians().sin().into()),
            Intrinsic::Cos => Ok(self.number(&arg(0))?.to_radians().cos().into()),
            Intrinsic::Tan => Ok(self.number(&arg(0))?.to_radians().tan().into()),
            Intrinsic::Arcsin => Ok(self.number(&arg(0))?.asin().to_degrees().into()),
            Intrinsic::Arccos => Ok(self.number(&arg(0))?.acos().to_degrees().into()),
            Intrinsic::Arctan => {
                let a = self.number(&arg(0))?;

                Ok(match args.len() {
                    0 | 1 => a.atan().to_degrees(),
                    _ => self.number(&arg(1))?.atan2(a).to_degrees(),
                }
                .into())
            },
            Intrinsic::Log => {
                let first = self.number(&arg(0))?;

                Ok(match args.len() {
                    0 | 1 => first.ln(),
                    _ => self.number(&arg(1))?.log(first),
                }
                .into())
            },
            Intrinsic::Sign => Ok(match self.number(&arg(0))? {
                n if n > 0.0 => 1.0,
                n if n < 0.0 => -1.0,
                _ => 0.0,
            }
            .into()),
            Intrinsic::Lerp => {
                let from = self.number(&arg(0))?;
                let to = self.number(&arg(1))?;
                let factor = self.number(&arg(2))?;

                Ok((from + (to - from) * factor).into())
            },
            Intrinsic::Fract => Ok(self.number(&arg(0))?.fract().into()),
            Intrinsic::Trunc => Ok(self.number(&arg(0))?.trunc().into()),
            Intrinsic::Isnan => Ok(self.number(&arg(0))?.is_nan().into()),
            Intrinsic::Isinf => Ok(self.number(&arg(0))?.is_infinite().into()),

            Intrinsic::Alist => {
                self.reserve(args.len())?;
                let entries = args
                    .iter()
                    .map(|(key, value)| match key {
                        Some(key) => (GenericValue::Text(key.as_str().into()), Some(value.clone())),
                        None => (value.clone(), None),
                    })
                    .collect::<Vec<_>>();

                self.list(entries)
            },

            Intrinsic::Alert
            | Intrinsic::Astype
            | Intrinsic::Block
            | Intrinsic::BoundPixloc
            | Intrinsic::BoundsDist
            | Intrinsic::Bounds
            | Intrinsic::DmDbClose
            | Intrinsic::DmDbColumns
            | Intrinsic::DmDbConnect
            | Intrinsic::DmDbErrorMsg
            | Intrinsic::DmDbExecute
            | Intrinsic::DmDbIsConnected
            | Intrinsic::DmDbNewCon
            | Intrinsic::DmDbNewQuery
            | Intrinsic::DmDbNextRow
            | Intrinsic::DmDbQuote
            | Intrinsic::DmDbRowCount
            | Intrinsic::DmDbRowsAffected
            | Intrinsic::Filter
            | Intrinsic::FindlasttextChar
            | Intrinsic::FindlasttextExChar
            | Intrinsic::FindtextChar
            | Intrinsic::FindtextExChar
            | Intrinsic::Ftime
            | Intrinsic::GetStepAway
            | Intrinsic::GetStepRand
            | Intrinsic::GetStepsTo
            | Intrinsic::GetStepTo
            | Intrinsic::GetStepTowards
            | Intrinsic::Generator
            | Intrinsic::Gradient
            | Intrinsic::Hearers
            | Intrinsic::HtmlDecode
            | Intrinsic::HtmlEncode
            | Intrinsic::IconStates
            | Intrinsic::Isfile
            | Intrinsic::Ispointer
            | Intrinsic::JsonEncode
            | Intrinsic::Lentext
            | Intrinsic::List2params
            | Intrinsic::LoadExt
            | Intrinsic::LoadResource
            | Intrinsic::Md5
            | Intrinsic::Missile
            | Intrinsic::Newlist
            | Intrinsic::NoiseHash
            | Intrinsic::NonspantextChar
            | Intrinsic::Nonspantext
            | Intrinsic::Obounds
            | Intrinsic::Ohearers
            | Intrinsic::Orange
            | Intrinsic::Output
            | Intrinsic::Oview
            | Intrinsic::Oviewers
            | Intrinsic::Params2list
            | Intrinsic::Pixloc
            | Intrinsic::RandSeed
            | Intrinsic::Range
            | Intrinsic::Refcount
            | Intrinsic::ReplacetextChar
            | Intrinsic::ReplacetextExChar
            | Intrinsic::Rgb2num
            | Intrinsic::Roll
            | Intrinsic::Run
            | Intrinsic::Sha1
            | Intrinsic::SpantextChar
            | Intrinsic::Spantext
            | Intrinsic::SplicetextChar
            | Intrinsic::Splicetext
            | Intrinsic::SplittextChar
            | Intrinsic::Startup
            | Intrinsic::Stat
            | Intrinsic::Statpanel
            | Intrinsic::StepAway
            | Intrinsic::StepRand
            | Intrinsic::Step
            | Intrinsic::StepTo
            | Intrinsic::StepTowards
            | Intrinsic::Text
            | Intrinsic::Time2text
            | Intrinsic::UrlDecode
            | Intrinsic::UrlEncode
            | Intrinsic::ValuesCutOver
            | Intrinsic::ValuesCutUnder
            | Intrinsic::ValuesDot
            | Intrinsic::ValuesProduct
            | Intrinsic::ValuesSum
            | Intrinsic::Vector
            | Intrinsic::View
            | Intrinsic::Viewers
            | Intrinsic::WalkAway
            | Intrinsic::WalkRand
            | Intrinsic::Walk
            | Intrinsic::WalkTo
            | Intrinsic::WalkTowards
            | Intrinsic::Winclone
            | Intrinsic::Winexists
            | Intrinsic::Winshow => Err(self.fault(FaultKind::Blocked(intrinsic.name().into()))),
        }
    }

    fn construct(
        &mut self, path: &str, params: &[Identifier], target: Option<ObjectId>,
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let id = match target {
            Some(id) => id,
            None => {
                let ty = self
                    .tree
                    .id_of(&TreePath::parse(path))
                    .ok_or_else(|| self.fault(FaultKind::MissingVariable(path.into())))?;
                self.reserve(1)?;

                self.runtime
                    .heap
                    .alloc_object(crate::heap::Object::new(ty))
                    .map_err(|kind| self.fault(kind))?
            },
        };

        let mut positional = 0;
        for (key, value) in args {
            let key = match key {
                Some(key) => key,
                None => {
                    let Some(name) = params.get(positional).cloned() else {
                        break;
                    };
                    positional += 1;

                    name
                },
            };

            self.write_field(GenericValue::Object(id), key, value)?;
        }

        Ok(GenericValue::Object(id))
    }

    fn resource_path(&mut self, file: &GenericValue) -> Result<Option<PathBuf>> {
        let Some(name) = file.text() else {
            return Ok(None);
        };
        let Some(root) = self.runtime.world.root.clone() else {
            return Err(self.fault(FaultKind::Blocked("filesystem".into())));
        };

        let path = root.join(name);
        match path.starts_with(&root) {
            true => Ok(Some(path)),
            false => Err(self.fault(FaultKind::Blocked("filesystem".into()))),
        }
    }

    fn file2list(&mut self, file: &GenericValue, separator: &GenericValue) -> Result<GenericValue> {
        let Some(path) = self.resource_path(file)? else {
            return Ok(GenericValue::Null);
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(GenericValue::Null);
        };
        if text.len() > self.limits.text_bytes {
            return Err(self.fault(FaultKind::Memory));
        }

        let separator = separator.text().unwrap_or("\n");
        let lines = match separator.is_empty() {
            true => vec![text.as_str()],
            false => text.split(separator).collect(),
        };

        self.reserve(lines.len())?;
        let entries = lines
            .iter()
            .map(|line| (GenericValue::Text(line.trim_end_matches('\r').into()), None))
            .collect::<Vec<_>>();

        self.runtime
            .heap
            .alloc_list(ListData::new(entries))
            .map(GenericValue::List)
            .map_err(|kind| self.fault(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::Intrinsic;

    const DEMIR: &str = include_str!("../../../dm/demir.dm");

    fn declared_ids() -> Vec<u32> {
        DEMIR
            .lines()
            .filter_map(|line| line.trim().strip_prefix("set __demir_intrin ="))
            .filter_map(|value| value.trim().parse::<u32>().ok())
            .collect()
    }

    /// A marker with no variant behind it reaches `FaultKind::Unsupported` at runtime instead of
    /// the `Blocked` the proc was written to give, and nothing else notices.
    #[test]
    fn every_prelude_marker_resolves_to_a_variant() {
        let ids = declared_ids();
        assert_eq!(ids.len(), Intrinsic::ALL.len());

        for id in ids {
            assert!(Intrinsic::from_id(id).is_some(), "demir.dm declares intrinsic {id}");
        }
    }

    /// A `/list` proc that is declared but never routed is the drift this table exists to stop:
    /// `RemoveAll` and `Splice` shipped declared-but-unimplemented under name matching.
    #[test]
    fn every_declared_list_proc_has_a_variant() {
        let declared = DEMIR
            .lines()
            .skip_while(|line| line.trim() != "/list")
            .take_while(|line| !line.starts_with("/alist"))
            .filter_map(|line| line.trim().strip_prefix("proc/"))
            .filter_map(|proc| proc.split("(").next())
            .filter(|name| *name != "New")
            .collect::<Vec<_>>();

        assert_eq!(declared.len(), 10);
        for name in declared {
            assert!(Intrinsic::list_proc(name).is_some(), "/list declares {name}");
        }
    }

    #[test]
    fn no_id_or_name_is_claimed_twice() {
        let mut ids = declared_ids();
        let declared = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), declared, "demir.dm reuses an intrinsic id");

        for intrinsic in Intrinsic::ALL {
            assert_eq!(Intrinsic::from_name(intrinsic.name()), Some(intrinsic));
        }
    }
}
