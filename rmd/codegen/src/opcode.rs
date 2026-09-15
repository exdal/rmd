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

byte_enum! {
    pub enum Op {
        Nop = 0x00,
        PushConstant = 0x01,
        LoadLocal = 0x02,
        StoreLocal = 0x03,
        LoadVariable = 0x04,
        StoreVariable = 0x05,
        PushBuiltin = 0x06,
        Interpolate = 0x07,
        Unary = 0x08,
        Binary = 0x09,
        AccessField = 0x0a,
        Index = 0x0b,
        Call = 0x0c,
        FunctionCall = 0x0d,
        SuperCall = 0x0e,
        New = 0x0f,
        ModifiedType = 0x10,
        MakeList = 0x11,
        Pick = 0x12,
        InRange = 0x13,
        Range = 0x14,
        IterInit = 0x15,
        IterNext = 0x16,
        IterValue = 0x17,
        IterKey = 0x18,
        RangeTest = 0x19,
        SetField = 0x1a,
        SetIndex = 0x1b,
        Jump = 0x1c,
        JumpIfFalse = 0x1d,
        Return = 0x1e,
        ReturnValue = 0x1f,
        Del = 0x20,
        Throw = 0x21,
        Output = 0x22,
        TryCatch = 0x23,
        Blocked = 0x24,
        Trap = 0x25,
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
        Dot = 0x07,
        Super = 0x08,
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

pub const ARGUMENT_KEY: u8 = 1 << 0;
pub const ARGUMENT_VALUE: u8 = 1 << 1;
