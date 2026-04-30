// vm/threaded.rs
//
// One pass that fuses LoadAttr+Call into CallMethod+CallMethodArgs.
//
// We mutate the instruction stream in-place rather than building a parallel
// enum. The fusion replaces the LoadAttr at index `i` with CallMethod (keeping
// the attr-name operand) and the Call at `i+1` with CallMethodArgs (keeping
// the call-args operand). All jump targets stay valid because we never change
// instruction count.
//
// Compile once per chunk, cache the result.

use crate::modules::parser::{OpCode, SSAChunk, Instruction};
use alloc::vec::Vec;

/// Build the dispatch-ready instruction stream for a chunk. Currently only
/// performs LoadAttr+Call fusion; cheap enough that it runs lazily on first
/// execution and the result is cached in `OpcodeCache`.
pub fn compile(chunk: &SSAChunk) -> Vec<Instruction> {
    let src = &chunk.instructions;
    let n = src.len();
    let mut out = src.clone();

    let mut i = 0;
    while i + 1 < n {
        if src[i].opcode == OpCode::LoadAttr && src[i + 1].opcode == OpCode::Call {
            // CallMethod carries the attr-name index; CallMethodArgs carries
            // the original Call's encoded (kw<<8 | pos) operand.
            out[i].opcode = OpCode::CallMethod;
            out[i + 1].opcode = OpCode::CallMethodArgs;
            i += 2;
            continue;
        }
        i += 1;
    }

    out
}