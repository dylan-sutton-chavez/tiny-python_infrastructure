// vm/handlers/unsupported.rs

use crate::modules::vm::VmErr;
use crate::modules::parser::OpCode;

// Specific message per opcode. Out-of-line + #[cold] -> out of the hot cache.
fn message(op: OpCode) -> &'static str {
    match op {
        OpCode::MakeClass => "classes not yet supported",
        OpCode::LoadAttr | OpCode::StoreAttr => "attribute access not yet supported",
        OpCode::SetupWith | OpCode::ExitWith => "with/as not yet supported",
        OpCode::UnpackArgs => "*args/**kwargs not yet supported",
        _ => "opcode not supported, please open an issue; https://github.com/dylan-sutton-chavez/edge-python/issues",
    }
}

#[cold]
pub(crate) fn unsupported(op: OpCode) -> VmErr {
    VmErr::Runtime(message(op))
}