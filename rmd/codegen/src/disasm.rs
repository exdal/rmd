use std::fmt::Write;

use crate::{
    CodeOffset,
    ConstantId,
    FunctionId,
    LocalId,
    Module,
    PathId,
    StringId,
    opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, Access, Binary, Builtin, Op, Unary},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    InvalidFunction(FunctionId),
    InvalidRange(FunctionId),
    Truncated(CodeOffset),
    InvalidOpcode {
        offset: CodeOffset,
        value: u8,
    },
    InvalidOperand {
        offset: CodeOffset,
        kind: &'static str,
        value: u8,
    },
    InvalidConstant(ConstantId),
    InvalidString(StringId),
    InvalidPath(PathId),
    InvalidFunctionOperand(FunctionId),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFunction(id) => write!(formatter, "bytecode function {id} does not exist"),
            Self::InvalidRange(id) => write!(formatter, "bytecode range for {id} is invalid"),
            Self::Truncated(offset) => write!(formatter, "truncated bytecode instruction at {offset}"),
            Self::InvalidOpcode { offset, value } => write!(formatter, "invalid opcode 0x{value:02x} at {offset}"),
            Self::InvalidOperand { offset, kind, value } => {
                write!(formatter, "invalid {kind} operand 0x{value:02x} at {offset}")
            },
            Self::InvalidConstant(id) => write!(formatter, "bytecode constant {id} does not exist"),
            Self::InvalidString(id) => write!(formatter, "bytecode string {id} does not exist"),
            Self::InvalidPath(id) => write!(formatter, "bytecode path {id} does not exist"),
            Self::InvalidFunctionOperand(id) => write!(formatter, "bytecode function operand {id} does not exist"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn dump(module: &Module) -> Result<String, DecodeError> {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "; bytecode module: {} bytes, {} constants, {} strings, {} paths, {} functions",
        module.code.len(),
        module.constants.len(),
        module.strings.len(),
        module.paths.len(),
        module.functions.len()
    );

    for function in &module.functions {
        let name = string(module, function.name)?;
        output.push('\n');
        if function.external {
            let _ = writeln!(output, "{} = external_function {name}", function.id);

            continue;
        }

        let _ = writeln!(
            output,
            "{} = function {name} params={} locals={} address={}",
            function.id, function.parameter_count, function.local_count, function.address
        );
        let start = function.address.0 as usize;
        let end = start
            .checked_add(function.length as usize)
            .filter(|end| *end <= module.code.len())
            .ok_or(DecodeError::InvalidRange(function.id))?;
        let mut cursor = Cursor {
            module,
            position: start,
            end,
            instruction: function.address,
        };
        while cursor.position < cursor.end {
            let offset = cursor.position;
            let instruction = cursor.instruction()?;
            let _ = writeln!(output, "  {} {instruction}", CodeOffset(offset as u32));
        }
    }

    Ok(output)
}

struct Cursor<'a> {
    module: &'a Module,
    position: usize,
    end: usize,
    instruction: CodeOffset,
}

impl Cursor<'_> {
    fn instruction(&mut self) -> Result<String, DecodeError> {
        self.instruction = CodeOffset(self.position as u32);
        let byte = self.u8()?;
        let op = Op::try_from(byte).map_err(|value| DecodeError::InvalidOpcode {
            offset: self.instruction,
            value,
        })?;
        let mut output = snake_case(op);

        match op {
            Op::Nop
            | Op::IterNext
            | Op::IterValue
            | Op::IterKey
            | Op::RangeTest
            | Op::SetIndex
            | Op::Return
            | Op::ReturnValue
            | Op::Del
            | Op::Throw
            | Op::Output => {},
            Op::PushConstant => {
                let id = ConstantId(self.u32()?);
                let value = self
                    .module
                    .constants
                    .get(id.0 as usize)
                    .ok_or(DecodeError::InvalidConstant(id))?;
                let _ = write!(output, " {id} ; {value}");
            },
            Op::LoadLocal | Op::StoreLocal => {
                let _ = write!(output, " {}", LocalId(self.u32()?));
            },
            Op::LoadVariable | Op::StoreVariable | Op::Blocked | Op::Trap => {
                let id = StringId(self.u32()?);
                let value = string(self.module, id)?;
                let _ = write!(output, " {value}");
            },
            Op::PushBuiltin => {
                let value = self.enum_operand::<Builtin>("builtin")?;
                let _ = write!(output, " {}", snake_case(value));
            },
            Op::Interpolate => {
                let values = self.u32()?;
                let chunk_count = self.u32()?;
                let mut chunks = Vec::with_capacity(chunk_count as usize);
                for _ in 0..chunk_count {
                    let id = StringId(self.u32()?);
                    chunks.push(format!("{id}=\"{}\"", string(self.module, id)?));
                }
                let _ = write!(output, " values={values} chunks=[{}]", chunks.join(", "));
            },
            Op::Unary => {
                let value = self.enum_operand::<Unary>("unary")?;
                let _ = write!(output, " {}", snake_case(value));
            },
            Op::Binary => {
                let value = self.enum_operand::<Binary>("binary")?;
                let _ = write!(output, " {}", snake_case(value));
            },
            Op::AccessField | Op::SetField => {
                let name = StringId(self.u32()?);
                let access = self.enum_operand::<Access>("access")?;
                let _ = write!(output, " {}{}", access_operator(access), string(self.module, name)?);
            },
            Op::Index => {
                if self.boolean()? {
                    output.push_str(" conditional");
                }
            },
            Op::Call | Op::SuperCall | Op::MakeList => {
                let layout = self.argument_layout()?;
                let _ = write!(output, " {layout}");
            },
            Op::FunctionCall => {
                let function = FunctionId(self.u32()?);
                if self.module.function(function).is_none() {
                    return Err(DecodeError::InvalidFunctionOperand(function));
                }
                let layout = self.argument_layout()?;
                let _ = write!(output, " {function} {layout}");
            },
            Op::New => {
                let has_type = self.boolean()?;
                let layout = self.argument_layout()?;
                if has_type {
                    output.push_str(" typed");
                }
                let _ = write!(output, " {layout}");
            },
            Op::ModifiedType => {
                let path = PathId(self.u32()?);
                let value = self
                    .module
                    .paths
                    .get(path.0 as usize)
                    .ok_or(DecodeError::InvalidPath(path))?;
                let count = self.u32()?;
                let mut names = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    names.push(string(self.module, StringId(self.u32()?))?);
                }
                let _ = write!(output, " {path}={value} overrides=[{}]", names.join(", "));
            },
            Op::Pick => {
                let count = self.u32()?;
                let mut choices = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    choices.push(match self.boolean()? {
                        true => "weighted",
                        false => "plain",
                    });
                }
                let _ = write!(output, " [{}]", choices.join(", "));
            },
            Op::InRange | Op::Range => {
                if self.boolean()? {
                    output.push_str(" step");
                }
            },
            Op::IterInit => {
                if self.boolean()? {
                    let path = PathId(self.u32()?);
                    let value = self
                        .module
                        .paths
                        .get(path.0 as usize)
                        .ok_or(DecodeError::InvalidPath(path))?;
                    let _ = write!(output, " ty={path} ; {value}");
                }
            },
            Op::Jump | Op::JumpIfFalse => {
                let _ = write!(output, " {}", CodeOffset(self.u32()?));
            },
            Op::TryCatch => {
                let body = CodeOffset(self.u32()?);
                let catch = CodeOffset(self.u32()?);
                let binding = self.u32()?;
                let _ = write!(output, " body={body} catch={catch}");
                if binding != u32::MAX {
                    let _ = write!(output, " binding=var{binding}");
                }
            },
        }

        Ok(output)
    }

    fn argument_layout(&mut self) -> Result<String, DecodeError> {
        let count = self.u32()?;
        let mut shapes = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let shape = self.u8()?;
            if shape & !(ARGUMENT_KEY | ARGUMENT_VALUE) != 0 {
                return Err(DecodeError::InvalidOperand {
                    offset: self.instruction,
                    kind: "argument shape",
                    value: shape,
                });
            }
            shapes.push(match shape {
                0 => "-",
                ARGUMENT_KEY => "key",
                ARGUMENT_VALUE => "value",
                _ => "key_value",
            });
        }

        Ok(format!("args=[{}]", shapes.join(", ")))
    }

    fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(DecodeError::InvalidOperand {
                offset: self.instruction,
                kind: "boolean",
                value,
            }),
        }
    }

    fn enum_operand<T>(&mut self, kind: &'static str) -> Result<T, DecodeError>
    where
        T: TryFrom<u8, Error = u8>,
    {
        let value = self.u8()?;
        T::try_from(value).map_err(|value| DecodeError::InvalidOperand {
            offset: self.instruction,
            kind,
            value,
        })
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        if self.position >= self.end {
            return Err(DecodeError::Truncated(self.instruction));
        }
        let value = self.module.code[self.position];
        self.position += 1;

        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let end = self
            .position
            .checked_add(4)
            .ok_or(DecodeError::Truncated(self.instruction))?;
        let bytes = self
            .module
            .code
            .get(self.position..end)
            .filter(|_| end <= self.end)
            .ok_or(DecodeError::Truncated(self.instruction))?;
        self.position = end;

        Ok(u32::from_le_bytes(bytes.try_into().expect("four-byte slice")))
    }
}

fn string(module: &Module, id: StringId) -> Result<&str, DecodeError> {
    module
        .strings
        .get(id.0 as usize)
        .map(String::as_str)
        .ok_or(DecodeError::InvalidString(id))
}

fn snake_case(value: impl std::fmt::Debug) -> String {
    let name = format!("{value:?}");
    let mut output = String::with_capacity(name.len());
    let mut previous_was_lowercase = false;

    for character in name.chars() {
        if character.is_ascii_uppercase() {
            if previous_was_lowercase {
                output.push('_');
            }
            output.push(character.to_ascii_lowercase());
            previous_was_lowercase = false;
        } else {
            output.push(character);
            previous_was_lowercase = character.is_ascii_lowercase() || character.is_ascii_digit();
        }
    }

    output
}

fn access_operator(access: Access) -> &'static str {
    match access {
        Access::Dot => ".",
        Access::Colon => ":",
        Access::SafeDot => "?.",
        Access::SafeColon => "?:",
        Access::Scope => "::",
    }
}
