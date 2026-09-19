use core::types::IrNodeId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenError {
    MissingNode(IrNodeId),
    ExpectedBlock(IrNodeId),
    ExpectedVariable(IrNodeId),
    MissingConstant(IrNodeId),
    MissingFunction(IrNodeId),
    MissingLocal(IrNodeId),
    MissingPhiOperand { phi: IrNodeId, predecessor: IrNodeId },
    CodeTooLarge,
    PoolTooLarge(&'static str),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingNode(id) => write!(formatter, "IR node {id} does not exist"),
            Self::ExpectedBlock(id) => write!(formatter, "IR node {id} is not a block"),
            Self::ExpectedVariable(id) => write!(formatter, "IR node {id} is not a variable"),
            Self::MissingConstant(id) => write!(formatter, "constant {id} is absent from the module constant pool"),
            Self::MissingFunction(id) => write!(formatter, "function {id} is absent from the bytecode function table"),
            Self::MissingLocal(id) => write!(formatter, "SSA value {id} has no bytecode local"),
            Self::MissingPhiOperand { phi, predecessor } => {
                write!(formatter, "phi {phi} has no value for predecessor block {predecessor}")
            },
            Self::CodeTooLarge => formatter.write_str("bytecode exceeds the 32-bit address space"),
            Self::PoolTooLarge(pool) => write!(formatter, "{pool} pool exceeds the 32-bit index space"),
        }
    }
}

impl std::error::Error for CodegenError {}
