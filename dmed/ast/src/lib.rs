use core::{
    location::Location,
    path::TreePath,
    types::{Identifier, VarModifiers},
};

use crate::parser::{ParseResult, Parser};

pub mod error;
pub mod parser;
pub mod precedence;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpressionId(u32);

impl ExpressionId {
    pub fn new(index: usize) -> Self { Self(u32::try_from(index).expect("expression arena exceeded u32::MAX")) }

    pub fn index(self) -> usize { self.0 as usize }
}

pub struct AST {
    pub declarations: Vec<Declaration>,
    pub expressions: Vec<Expression>,
}

impl AST {
    pub fn new(declarations: Vec<Declaration>, expressions: Vec<Expression>) -> Self {
        Self {
            declarations,
            expressions,
        }
    }

    pub fn get_expr(&self, expr_id: ExpressionId) -> Option<&Expression> { self.expressions.get(expr_id.index()) }
}

#[derive(Debug, Clone)]
pub enum Declaration {
    Type {
        path: TreePath,
        body: Vec<Declaration>,
        location: Location,
    },

    Var {
        path: TreePath,
        var_type: Option<TreePath>,
        modifiers: VarModifiers,
        dimensions: Vec<Option<ExpressionId>>,
        as_type: Option<TypeSpec>,
        in_list: Option<ExpressionId>,
        initializer: Option<ExpressionId>,
        location: Location,
    },

    Proc {
        path: TreePath,
        kind: ProcKind,
        params: Vec<ProcParam>,
        variadic: bool,
        return_type: Option<TypeSpec>,
        body: Option<Vec<Statement>>,
        location: Location,
    },

    /// `icon_state = "sword"`
    Override {
        name: Identifier,
        value: ExpressionId,
        location: Location,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcKind {
    Proc,
    Verb,
    Override,
    /// `operator+`, `operator[]`, ...
    Operator,
}

#[derive(Debug, Clone)]
pub struct ProcParam {
    pub spec: VarSpec,
    pub default: Option<ExpressionId>,
    /// `in list(...)`
    pub in_list: Option<ExpressionId>,
}

#[derive(Default, Debug, Clone)]
pub struct TypeSpec {
    pub flags: InputType,
    pub path: Option<TreePath>,
}

#[derive(Debug, Clone)]
pub struct VarSpec {
    pub name: Identifier,
    pub var_type: Option<TreePath>,
    pub modifiers: VarModifiers,
    /// `var/items[5][]`
    pub dimensions: Vec<Option<ExpressionId>>,
    /// `as text|null` or `as /mob/player`
    pub as_type: Option<TypeSpec>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputType(pub u32);

impl InputType {
    pub const ANYTHING: Self = Self(1 << 5);
    pub const AREA: Self = Self(1 << 13);
    pub const COLOR: Self = Self(1 << 7);
    pub const COMMAND_TEXT: Self = Self(1 << 8);
    pub const FILE: Self = Self(1 << 3);
    pub const ICON: Self = Self(1 << 6);
    pub const KEY: Self = Self(1 << 4);
    pub const MESSAGE: Self = Self(1 << 9);
    pub const MOB: Self = Self(1 << 1);
    pub const NONE: Self = Self(0);
    pub const NULL: Self = Self(1 << 10);
    pub const NUM: Self = Self(1 << 0);
    pub const OBJ: Self = Self(1 << 2);
    pub const PASSWORD: Self = Self(1 << 14);
    pub const PATH: Self = Self(1 << 16);
    pub const SOUND: Self = Self(1 << 15);
    pub const TEXT: Self = Self(1 << 11);
    pub const TURF: Self = Self(1 << 12);

    pub fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }

    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "anything" => Self::ANYTHING,
            "area" => Self::AREA,
            "color" => Self::COLOR,
            "command_text" => Self::COMMAND_TEXT,
            "file" => Self::FILE,
            "icon" => Self::ICON,
            "key" => Self::KEY,
            "message" => Self::MESSAGE,
            "mob" => Self::MOB,
            "null" => Self::NULL,
            "num" => Self::NUM,
            "obj" => Self::OBJ,
            "password" => Self::PASSWORD,
            "path" => Self::PATH,
            "sound" => Self::SOUND,
            "text" => Self::TEXT,
            "turf" => Self::TURF,
            _ => return None,
        })
    }
}

impl std::ops::BitOr for InputType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
}

impl std::ops::BitOrAssign for InputType {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}

#[derive(Debug, Clone)]
pub enum Statement {
    Empty,

    Expression(ExpressionId),

    /// `A << B`
    Output {
        target: ExpressionId,
        value: ExpressionId,
    },

    /// `A >> B`
    Input {
        source: ExpressionId,
        destination: ExpressionId,
    },

    Var {
        spec: VarSpec,
        initializer: Option<ExpressionId>,
        in_list: Option<ExpressionId>,
    },

    Return(Option<ExpressionId>),

    If {
        branches: Vec<(ExpressionId, Vec<Statement>)>,
        else_branch: Option<Vec<Statement>>,
    },

    While {
        condition: ExpressionId,
        body: Vec<Statement>,
    },

    DoWhile {
        body: Vec<Statement>,
        condition: ExpressionId,
    },

    For(Box<ForLoop>),

    Switch {
        value: ExpressionId,
        cases: Vec<SwitchCase>,
        default: Option<Vec<Statement>>,
    },

    /// `spawn(5)`
    Spawn {
        delay: Option<ExpressionId>,
        body: Vec<Statement>,
    },

    TryCatch {
        try_body: Vec<Statement>,
        catch_param: Option<VarSpec>,
        catch_body: Vec<Statement>,
    },

    Throw(ExpressionId),
    Del(ExpressionId),

    /// `set waitfor = FALSE`, `set name = "..."`, `set src in view(1)`
    Setting {
        name: Identifier,
        mode: SettingMode,
        value: ExpressionId,
    },

    Break(Option<Identifier>),
    Continue(Option<Identifier>),
    Goto(Identifier),
    Label {
        name: Identifier,
        body: Vec<Statement>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingMode {
    /// `set x = y`
    Assign,
    /// `set x in y`
    In,
}

#[derive(Debug, Clone)]
pub enum ForLoop {
    /// `for(var/i = 0, i < 10, i++)`
    Standard {
        init: Option<Box<Statement>>,
        condition: Option<ExpressionId>,
        step: Option<Box<Statement>>,
        body: Vec<Statement>,
    },

    /// `for(var/mob/M in world)`
    List {
        key: Option<LoopBinding>,
        value: LoopBinding,
        /// `for(var/client/C)`
        list: Option<ExpressionId>,
        body: Vec<Statement>,
    },

    /// `for(var/i = 1 to 10 step 2)`
    Range {
        variable: LoopBinding,
        start: ExpressionId,
        end: ExpressionId,
        step: Option<ExpressionId>,
        body: Vec<Statement>,
    },
}

#[derive(Debug, Clone)]
pub struct LoopBinding {
    pub spec: VarSpec,
    pub declares: bool,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub values: Vec<SwitchValue>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum SwitchValue {
    Value(ExpressionId),
    /// `if(1 to 5)`
    Range(ExpressionId, ExpressionId),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Literal(Literal),
    Identifier(Identifier),
    /// `/obj/item/sword`
    Path(TreePath),
    /// `src`, `usr`, `world`, `global`, `args`
    Builtin(Builtin),

    /// `f((a = 1))`
    Grouped(ExpressionId),

    /// `"you have [n] credits"`
    InterpString {
        chunks: Vec<String>,
        /// "a[]b"`, `"a[/* c */]b"`
        /// dude what the fuck
        expressions: Vec<Option<ExpressionId>>,
    },

    Unary {
        op: UnaryOp,
        operand: ExpressionId,
    },

    Binary {
        op: BinaryOp,
        lhs_expr: ExpressionId,
        rhs_expr: ExpressionId,
    },

    /// `value in start to end step step`
    InRange {
        value: ExpressionId,
        start: ExpressionId,
        end: ExpressionId,
        step: Option<ExpressionId>,
    },

    /// `1 to 10 step 2`
    Range {
        start: ExpressionId,
        end: ExpressionId,
        step: Option<ExpressionId>,
    },

    Assign {
        kind: AssignmentKind,
        lhs_expr: ExpressionId,
        rhs_expr: ExpressionId,
    },

    Ternary {
        condition: ExpressionId,
        true_expr: ExpressionId,
        false_expr: ExpressionId,
    },

    /// `a.b`, `a:b`, `a?.b`
    Field {
        object: ExpressionId,
        name: Identifier,
        access: AccessKind,
    },

    /// `a[i]`, `a?[i]`
    Index {
        object: ExpressionId,
        index: ExpressionId,
        conditional: bool,
    },

    Call {
        callee: ExpressionId,
        args: Vec<Argument>,
    },

    /// `new /obj/item(loc)`, `new(loc)`
    New {
        type_expr: Option<ExpressionId>,
        args: Vec<Argument>,
    },

    /// `/obj/item { name = "custom" }`
    ModifiedType {
        path: TreePath,
        overrides: Vec<(Identifier, ExpressionId)>,
    },

    /// `list(1, 2, "a" = 3)`
    List(Vec<Argument>),

    Pick(Vec<PickValue>),

    /// `input(usr, "pick") as text`
    Input {
        args: Vec<Argument>,
        input_type: InputType,
        in_list: Option<ExpressionId>,
    },
}

#[derive(Debug, Clone)]
pub struct Argument {
    pub key: Option<ExpressionId>,
    pub value: Option<ExpressionId>,
}

#[derive(Debug, Clone)]
pub struct PickValue {
    /// `pick(10; "rare", "common")`
    pub weight: Option<ExpressionId>,
    pub value: ExpressionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// `.`
    Dot,
    /// `:`
    Colon,
    /// `?.`
    SafeDot,
    /// `?:`
    SafeColon,
    /// `::`
    Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Src,
    Usr,
    World,
    Global,
    Args,
    Callee,
    Caller,
    /// `.`
    Dot,
    /// `..()`
    Super,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Num(f32),
    String(String),
    Resource(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    PreIncrement,
    PreDecrement,
    PostIncrement,
    PostDecrement,
    Reference,
    Dereference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentKind {
    Assign,
    CompoundAdd,
    CompoundSub,
    CompoundMul,
    CompoundDiv,
    CompoundMod,
    CompoundFloatMod,
    CompoundPow,
    CompoundAnd,
    CompoundOr,
    CompoundXor,
    CompoundShl,
    CompoundShr,
    LogicalAndSet,
    LogicalOrSet,
    AssignInto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    FloatMod,
    Pow,

    BitAnd,
    BitXor,
    BitOr,

    CompGreater,
    CompLess,
    CompEq,
    CompNotEq,
    CompGreaterEq,
    CompLessEq,
    CompEquiv,
    CompNotEquiv,
    CompThreeWay,

    LogicalAnd,
    LogicalOr,

    ShiftLeft,
    ShiftRight,

    In,
}

pub fn parse(tokens: &[(lexer::token::Token<'_>, Location)]) -> ParseResult<AST> {
    let mut parser = Parser::new(tokens);

    parser.parse()
}
