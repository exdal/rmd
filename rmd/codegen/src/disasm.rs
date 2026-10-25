use std::fmt::Write;

use crate::{
    CodeOffset, ConstantId, FunctionId, Module, PathId, RegisterId, StringId,
    opcode::{ARGUMENT_KEY, ARGUMENT_VALUE, Access, Binary, Builtin, Op, OutputTargetKind, Unary},
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

        let intrinsic = function
            .intrinsic
            .map(|intrinsic| format!(" intrinsic={intrinsic}"))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "{} = function {name} params={} registers={} address={}{}",
            function.id, function.parameter_count, function.register_count, function.address, intrinsic
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
            Op::Nop | Op::Return | Op::ReturnNull | Op::EndSpawn => {},
            Op::MoveConst => {
                let destination = self.register()?;
                let id = ConstantId(self.u32()?);
                let value = self
                    .module
                    .constants
                    .get(id.0 as usize)
                    .ok_or(DecodeError::InvalidConstant(id))?;
                let _ = write!(output, " {destination}, {id} ; {value}");
            },
            Op::MoveImm => {
                let destination = self.register()?;
                let _ = write!(output, " {destination}, {}", self.f32()?);
            },
            Op::Move | Op::DefaultParameter | Op::IterNext | Op::IterValue | Op::IterKey => {
                let destination = self.register()?;
                let source = self.register()?;
                let _ = write!(output, " {destination}, {source}");
            },
            Op::ReturnValue | Op::Del | Op::Throw | Op::CatchValue => {
                let _ = write!(output, " {}", self.register()?);
            },
            Op::ReturnValueI => {
                let _ = write!(output, " {}", self.f32()?);
            },
            Op::LoadVar => {
                let destination = self.register()?;
                let value = string(self.module, StringId(self.u32()?))?;
                let _ = write!(output, " {destination}, {value}");
            },
            Op::StoreVar | Op::InitVar => {
                let value = string(self.module, StringId(self.u32()?))?;
                let source = self.register()?;
                let _ = write!(output, " {value}, {source}");
            },
            Op::Blocked | Op::Trap => {
                let id = StringId(self.u32()?);
                let value = string(self.module, id)?;
                let _ = write!(output, " {value}");
            },
            Op::LoadBuiltin => {
                let destination = self.register()?;
                let value = self.enum_operand::<Builtin>("builtin")?;
                let _ = write!(output, " {destination}, {}", snake_case(value));
            },
            Op::StoreBuiltin => {
                let value = self.enum_operand::<Builtin>("builtin")?;
                let source = self.register()?;
                let _ = write!(output, " {}, {source}", snake_case(value));
            },
            Op::Interpolate => {
                let destination = self.register()?;
                let value_count = self.u32()?;
                let chunk_count = self.u32()?;
                let mut chunks = Vec::with_capacity(chunk_count as usize);
                for _ in 0..chunk_count {
                    let id = StringId(self.u32()?);
                    chunks.push(format!("{id}=\"{}\"", string(self.module, id)?));
                }

                let values = self.registers(value_count)?;
                let _ = write!(
                    output,
                    " {destination}, chunks=[{}] values=[{values}]",
                    chunks.join(", ")
                );
            },
            Op::Unary => {
                let destination = self.register()?;
                let value = self.enum_operand::<Unary>("unary")?;
                let source = self.register()?;
                let _ = write!(output, " {destination}, {} {source}", snake_case(value));
            },
            Op::Binary | Op::CompoundBinary => {
                let destination = self.register()?;
                let value = self.enum_operand::<Binary>("binary")?;
                let left = self.register()?;
                let right = self.register()?;
                let _ = write!(output, " {destination}, {} {left}, {right}", snake_case(value));
            },
            Op::BinaryI | Op::CompoundBinaryI => {
                let destination = self.register()?;
                let value = self.enum_operand::<Binary>("binary")?;
                let left = self.register()?;
                let right = self.f32()?;
                let _ = write!(output, " {destination}, {} {left}, {right}", snake_case(value));
            },
            Op::AccessField => {
                let destination = self.register()?;
                let name = StringId(self.u32()?);
                let access = self.enum_operand::<Access>("access")?;
                let is_method = self.boolean()?;
                let object = self.register()?;
                let _ = write!(
                    output,
                    " {destination}, {object}{}{}",
                    access_operator(access),
                    string(self.module, name)?
                );
                if is_method {
                    output.push_str(" method");
                }
            },
            Op::SetField => {
                let name = StringId(self.u32()?);
                let access = self.enum_operand::<Access>("access")?;
                let object = self.register()?;
                let value = self.register()?;
                let _ = write!(
                    output,
                    " {object}{}{}, {value}",
                    access_operator(access),
                    string(self.module, name)?
                );
            },
            Op::Initial => {
                let destination = self.register()?;
                let has_object = self.boolean()?;
                let name = string(self.module, StringId(self.u32()?))?;
                if has_object {
                    let object = self.register()?;
                    let _ = write!(output, " {destination}, {object}.{name}");
                } else {
                    let _ = write!(output, " {destination}, {name}");
                }
            },
            Op::Index => {
                let destination = self.register()?;
                let is_conditional = self.boolean()?;
                let object = self.register()?;
                let index = self.register()?;
                let _ = write!(output, " {destination}, {object}[{index}]");
                if is_conditional {
                    output.push_str(" conditional");
                }
            },
            Op::IndexI => {
                let destination = self.register()?;
                let is_conditional = self.boolean()?;
                let object = self.register()?;
                let index = self.f32()?;
                let _ = write!(output, " {destination}, {object}[{index}]");
                if is_conditional {
                    output.push_str(" conditional");
                }
            },
            Op::SetIndex => {
                let is_conditional = self.boolean()?;
                let object = self.register()?;
                let index = self.register()?;
                let value = self.register()?;
                let _ = write!(output, " {object}[{index}], {value}");
                if is_conditional {
                    output.push_str(" conditional");
                }
            },
            Op::Output => match self.enum_operand::<OutputTargetKind>("output target")? {
                OutputTargetKind::Value => {
                    let target = self.register()?;
                    let value = self.register()?;
                    let _ = write!(output, " value {target}, {value}");
                },
                OutputTargetKind::Field => {
                    let name = StringId(self.u32()?);
                    let access = self.enum_operand::<Access>("access")?;
                    let object = self.register()?;
                    let value = self.register()?;
                    let _ = write!(
                        output,
                        " field {object}{}{}, {value}",
                        access_operator(access),
                        string(self.module, name)?
                    );
                },
                OutputTargetKind::Index => {
                    let is_conditional = self.boolean()?;
                    let object = self.register()?;
                    let index = self.register()?;
                    let value = self.register()?;
                    let _ = write!(output, " index {object}[{index}], {value}");
                    if is_conditional {
                        output.push_str(" conditional");
                    }
                },
            },
            Op::MakeList => {
                let destination = self.register()?;
                let layout = self.argument_layout()?;
                let _ = write!(output, " {destination}, {layout}");
            },
            Op::Call => {
                let destination = self.register()?;
                let is_conditional = self.boolean()?;
                let callee = self.register()?;
                let layout = self.argument_layout()?;
                let _ = write!(output, " {destination}, {callee} {layout}");
                if is_conditional {
                    output.push_str(" conditional");
                }
            },
            Op::SuperCall => {
                let destination = self.register()?;
                let forwards_extra_args = self.boolean()?;
                let layout = self.argument_layout()?;
                let _ = write!(output, " {destination}, {layout}");
                if forwards_extra_args {
                    output.push_str(" extra_args");
                }
            },
            Op::FunctionCall => {
                let destination = self.register()?;
                let function = FunctionId(self.u32()?);
                if self.module.function(function).is_none() {
                    return Err(DecodeError::InvalidFunctionOperand(function));
                }

                let layout = self.argument_layout()?;
                let _ = write!(output, " {destination}, {function} {layout}");
            },
            Op::New => {
                let destination = self.register()?;
                let has_type = self.boolean()?;
                let _ = write!(output, " {destination},");
                if has_type {
                    let _ = write!(output, " {}", self.register()?);
                }

                let layout = self.argument_layout()?;
                let _ = write!(output, " {layout}");
            },
            Op::ModifiedType => {
                let destination = self.register()?;
                let path = PathId(self.u32()?);
                let value = self
                    .module
                    .paths
                    .get(path.0 as usize)
                    .ok_or(DecodeError::InvalidPath(path))?;
                let count = self.u32()?;
                let mut overrides = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let name = string(self.module, StringId(self.u32()?))?;
                    overrides.push(format!("{name}={}", self.register()?));
                }

                let _ = write!(
                    output,
                    " {destination}, {path}={value} overrides=[{}]",
                    overrides.join(", ")
                );
            },
            Op::Pick => {
                let destination = self.register()?;
                let count = self.u32()?;
                let mut choices = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let choice = if self.boolean()? {
                        let weight = self.register()?;
                        format!("{weight}: {}", self.register()?)
                    } else {
                        self.register()?.to_string()
                    };
                    choices.push(choice);
                }

                let _ = write!(output, " {destination}, [{}]", choices.join(", "));
            },
            Op::InRange => {
                let destination = self.register()?;
                let has_step = self.boolean()?;
                let value = self.register()?;
                let start = self.register()?;
                let end = self.register()?;
                let _ = write!(output, " {destination}, {value} in {start} to {end}");
                if has_step {
                    let _ = write!(output, " step {}", self.register()?);
                }
            },
            Op::Range => {
                let destination = self.register()?;
                let has_step = self.boolean()?;
                let start = self.register()?;
                let end = self.register()?;
                let _ = write!(output, " {destination}, {start} to {end}");
                if has_step {
                    let _ = write!(output, " step {}", self.register()?);
                }
            },
            Op::RangeTest => {
                let destination = self.register()?;
                let current = self.register()?;
                let end = self.register()?;
                let step = self.register()?;
                let _ = write!(output, " {destination}, {current}, {end}, {step}");
            },
            Op::IterInit => {
                let destination = self.register()?;
                let _ = write!(output, " {destination},");
                if self.boolean()? {
                    let path = PathId(self.u32()?);
                    let value = self
                        .module
                        .paths
                        .get(path.0 as usize)
                        .ok_or(DecodeError::InvalidPath(path))?;
                    let _ = write!(output, " ty={path}={value}");
                }

                let is_associated = self.boolean()?;
                let _ = write!(output, " {}", self.register()?);
                if is_associated {
                    output.push_str(" values=associated");
                }
            },
            Op::Jump => {
                let _ = write!(output, " {}", CodeOffset(self.u32()?));
            },
            Op::JumpIfFalse => {
                let target = CodeOffset(self.u32()?);
                let condition = self.register()?;
                let _ = write!(output, " {target}, {condition}");
            },
            Op::TryCatch => {
                let body = CodeOffset(self.u32()?);
                let catch = CodeOffset(self.u32()?);
                let merge = CodeOffset(self.u32()?);
                let _ = write!(output, " body={body} catch={catch} merge={merge}");
            },
            Op::Spawn => {
                let body = CodeOffset(self.u32()?);
                let _ = write!(output, " body={body}");
            },
        }

        Ok(output)
    }

    fn registers(&mut self, count: u32) -> Result<String, DecodeError> {
        let registers = (0..count)
            .map(|_| self.register().map(|register| register.to_string()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(registers.join(", "))
    }

    fn argument_layout(&mut self) -> Result<String, DecodeError> {
        let count = self.u32()?;
        let mut args = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let shape = self.u8()?;
            if shape & !(ARGUMENT_KEY | ARGUMENT_VALUE) != 0 {
                return Err(DecodeError::InvalidOperand {
                    offset: self.instruction,
                    kind: "argument shape",
                    value: shape,
                });
            }

            let key = if shape & ARGUMENT_KEY != 0 {
                Some(self.register()?)
            } else {
                None
            };
            let value = if shape & ARGUMENT_VALUE != 0 {
                Some(self.register()?)
            } else {
                None
            };
            args.push(match (key, value) {
                (Some(key), Some(value)) => format!("{key}={value}"),
                (Some(key), None) => format!("{key}="),
                (None, Some(value)) => value.to_string(),
                (None, None) => "-".to_owned(),
            });
        }

        Ok(format!("args=[{}]", args.join(", ")))
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

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let end = self
            .position
            .checked_add(2)
            .ok_or(DecodeError::Truncated(self.instruction))?;
        let bytes = self
            .module
            .code
            .get(self.position..end)
            .filter(|_| end <= self.end)
            .ok_or(DecodeError::Truncated(self.instruction))?;
        self.position = end;

        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn register(&mut self) -> Result<RegisterId, DecodeError> {
        self.u16().map(RegisterId)
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

    fn f32(&mut self) -> Result<f32, DecodeError> {
        self.u32().map(f32::from_bits)
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
