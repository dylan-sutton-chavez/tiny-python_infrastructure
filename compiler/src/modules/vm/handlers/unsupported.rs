// vm/handlers/unsupported.rs

use crate::modules::vm::VmErr;
use crate::modules::parser::OpCode;

// Specific message per opcode. Out-of-line + #[cold] -> out of the hot cache.
fn message(op: OpCode) -> &'static str {
    match op {
        OpCode::MakeClass | OpCode::StoreAttr => "objects not yet supported",
        _ => "opcode not yet supported",
    }
}

#[cold]
pub(crate) fn unsupported(op: OpCode) -> VmErr {
    VmErr::Runtime(message(op))
}