macro_rules! byte_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident = $value:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant = $value,)*
        }

        impl TryFrom<u8> for $name {
            type Error = u8;

            fn try_from(value: u8) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant),)*
                    _ => Err(value),
                }
            }
        }
    };
}

// operand order: rD, immediates, source registers. An imm sits where the source register it replaces would.
// r = u16 register, k = u32 constant, name/str/path/fn/count/target/slot = u32, flag/op = u8, imm = f32
// args = count, then per argument: shape:u8, rKey if ARGUMENT_KEY, rValue if ARGUMENT_VALUE
byte_enum! {
    pub enum Op {
        Nop = 0x00,
        MoveConst = 0x01,        // rD, k
        Move = 0x02,             // rD, rS
        LoadVar = 0x04,          // rD, name
        StoreVar = 0x05,         // name, rS
        LoadBuiltin = 0x06,      // rD, builtin:u8
        Interpolate = 0x07,      // rD, value_count, chunk_count, str * chunk_count, r * value_count
        Unary = 0x08,            // rD, op:Unary, rS
        Binary = 0x09,           // rD, op:Binary, rA, rB
        AccessField = 0x0a,      // rD, name, access:Access, method:flag, rObject
        Index = 0x0b,            // rD, conditional:flag, rObject, rIndex
        Call = 0x0c,             // rD, conditional:flag, rCallee, args
        FunctionCall = 0x0d,     // rD, fn, args
        SuperCall = 0x0e,        // rD, forwards_extra_args:flag, args
        New = 0x0f,              // rD, has_type:flag, [rType], args
        ModifiedType = 0x10,     // rD, path, count, (name, rValue) * count
        MakeList = 0x11,         // rD, args
        Pick = 0x12,             // rD, count, (weighted:flag, [rWeight], rValue) * count
        InRange = 0x13,          // rD, has_step:flag, rValue, rStart, rEnd, [rStep]
        Range = 0x14,            // rD, has_step:flag, rStart, rEnd, [rStep]
        IterInit = 0x15,         // rD, has_type:flag, [path], associated:flag, rList
        IterNext = 0x16,         // rD, rIter
        IterValue = 0x17,        // rD, rIter
        IterKey = 0x18,          // rD, rIter
        RangeTest = 0x19,        // rD, rCurrent, rEnd, rStep
        SetField = 0x1a,         // name, access:Access, rObject, rValue
        SetIndex = 0x1b,         // conditional:flag, rObject, rIndex, rValue
        Jump = 0x1c,             // target
        JumpIfFalse = 0x1d,      // target, rCondition
        Return = 0x1e,           //
        ReturnValue = 0x1f,      // rS
        Del = 0x20,              // rS
        Throw = 0x21,            // rS
        Output = 0x22,           // kind:OutputTargetKind, then by kind:
                                 //   Value: rTarget, rValue
                                 //   Field: name, access:Access, rObject, rValue
                                 //   Index: conditional:flag, rObject, rIndex, rValue
        TryCatch = 0x23,         // body:target, catch:target
        Blocked = 0x24,          // reason:str
        Trap = 0x25,             // reason:str
        StoreBuiltin = 0x26,     // builtin:u8, rS
        InitVar = 0x27,          // name, rS
        CatchValue = 0x28,       // rD
        DefaultParameter = 0x29, // rParameter, rValue, writes rValue only while rParameter is omitted
        CompoundBinary = 0x2a,   // rD, op:Binary, rA, rB
        Initial = 0x2b,          // rD, has_object:flag, name, [rObject]
        Spawn = 0x2d,            // body:target
        EndSpawn = 0x2e,         //
        BinaryI = 0x2f,          // rD, op:Binary, rA, imm
        CompoundBinaryI = 0x30,  // rD, op:Binary, rA, imm
        IndexI = 0x31,           // rD, conditional:flag, rObject, imm
        ReturnValueI = 0x32,     // imm
        ReturnNull = 0x33,       //
        MoveImm = 0x34,          // rD, imm
        Leave = 0x35,            // regions:u32, target
        LoadSlot = 0x36,         // rD, slot
        StoreSlot = 0x37,        // slot, rS
    }
}

byte_enum! {
    pub enum Unary {
        Neg = 0x00,
        Not = 0x01,
        BitNot = 0x02,
        PreIncrement = 0x03,
        PreDecrement = 0x04,
        PostIncrement = 0x05,
        PostDecrement = 0x06,
        Reference = 0x07,
        Dereference = 0x08,
    }
}

byte_enum! {
    pub enum Binary {
        Add = 0x00,
        Sub = 0x01,
        Mul = 0x02,
        Div = 0x03,
        Mod = 0x04,
        FloatMod = 0x05,
        Pow = 0x06,
        BitAnd = 0x07,
        BitXor = 0x08,
        BitOr = 0x09,
        CompGreater = 0x0a,
        CompLess = 0x0b,
        CompEq = 0x0c,
        CompNotEq = 0x0d,
        CompGreaterEq = 0x0e,
        CompLessEq = 0x0f,
        CompEquiv = 0x10,
        CompNotEquiv = 0x11,
        CompThreeWay = 0x12,
        LogicalAnd = 0x13,
        LogicalOr = 0x14,
        ShiftLeft = 0x15,
        ShiftRight = 0x16,
        In = 0x17,
    }
}

byte_enum! {
    pub enum Builtin {
        Src = 0x00,
        Usr = 0x01,
        World = 0x02,
        Global = 0x03,
        Args = 0x04,
        Callee = 0x05,
        Caller = 0x06,
        ThisProc = 0x07,
        SuperProc = 0x08,
    }
}

byte_enum! {
    pub enum Access {
        Dot = 0x00,
        Colon = 0x01,
        SafeDot = 0x02,
        SafeColon = 0x03,
        Scope = 0x04,
    }
}

byte_enum! {
    pub enum OutputTargetKind {
        Value = 0x00,
        Field = 0x01,
        Index = 0x02,
    }
}

pub const ARGUMENT_KEY: u8 = 1 << 0;
pub const ARGUMENT_VALUE: u8 = 1 << 1;
