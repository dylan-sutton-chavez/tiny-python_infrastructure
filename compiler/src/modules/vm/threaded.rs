// vm/threaded.rs

use crate::modules::parser::{OpCode, SSAChunk};

use alloc::vec::Vec;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum ThreadedOp {
    // ── stack-mode hot path ───────────────────────────────────────
    LoadName(u16),
    StoreName(u16),
    LoadConst(u16),
    LoadTrue, LoadFalse, LoadNone,

    Add, Sub, Mul, Div, Mod, Pow, FloorDiv, Minus,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    Not,

    Jump(u16),
    JumpIfFalse(u16),
    JumpIfFalseOrPop(u16),
    JumpIfTrueOrPop(u16),
    ForIter(u16),
    GetIter(u16),

    PopTop,
    ReturnValue,

    Call(u16),
    CallPrint(u16), CallLen, CallAbs, CallStr, CallInt, CallFloat,
    CallBool, CallType, CallChr, CallOrd, CallSorted, CallList,
    CallTuple, CallEnumerate, CallIsInstance, CallRange(u16),
    CallRound(u16), CallMin(u16), CallMax(u16), CallSum(u16),
    CallZip(u16), CallDict(u16), CallSet(u16), CallInput,
    MakeFunction(u16), MakeCoroutine(u16),

    GetItem,
    LoadAttr(u16),

    Phi { operand: u16, rip: usize },

    /// Fused LoadAttr+Call: resolves method and calls it without heap-allocating BoundMethod.
    CallMethod { attr_idx: u16, call_op: u16 },

    /// Unreachable (And/Or should be short-circuited by parser)
    Unreachable,
    /// No-op placeholder for fused instructions (keeps jump targets aligned)
    Nop,

    /// Cold opcodes that don't benefit from dedicated variants.
    /// Falls through to the original OpCode dispatch.
    Generic { opcode: OpCode, operand: u16 },
}

/// Compile a chunk's instruction stream into threaded ops.
pub fn compile(chunk: &SSAChunk) -> Vec<ThreadedOp> {
    let ins = &chunk.instructions;
    let mut ops = Vec::with_capacity(ins.len());
    let mut ip = 0;

    while ip < ins.len() {
        let i = &ins[ip];
        let op = i.operand;

        // Fuse LoadAttr + Call → CallMethod
        if i.opcode == OpCode::LoadAttr
            && ip + 1 < ins.len()
            && ins[ip + 1].opcode == OpCode::Call
        {
            ops.push(ThreadedOp::CallMethod { attr_idx: op, call_op: ins[ip + 1].operand });
            ops.push(ThreadedOp::Nop);
            ip += 2;
            continue;
        }

        ops.push(match i.opcode {
            OpCode::LoadName    => ThreadedOp::LoadName(op),
            OpCode::StoreName   => ThreadedOp::StoreName(op),
            OpCode::LoadConst   => ThreadedOp::LoadConst(op),
            OpCode::LoadTrue    => ThreadedOp::LoadTrue,
            OpCode::LoadFalse   => ThreadedOp::LoadFalse,
            OpCode::LoadNone    => ThreadedOp::LoadNone,

            OpCode::Add         => ThreadedOp::Add,
            OpCode::Sub         => ThreadedOp::Sub,
            OpCode::Mul         => ThreadedOp::Mul,
            OpCode::Div         => ThreadedOp::Div,
            OpCode::Mod         => ThreadedOp::Mod,
            OpCode::Pow         => ThreadedOp::Pow,
            OpCode::FloorDiv    => ThreadedOp::FloorDiv,
            OpCode::Minus       => ThreadedOp::Minus,

            OpCode::Eq          => ThreadedOp::Eq,
            OpCode::NotEq       => ThreadedOp::NotEq,
            OpCode::Lt          => ThreadedOp::Lt,
            OpCode::Gt          => ThreadedOp::Gt,
            OpCode::LtEq        => ThreadedOp::LtEq,
            OpCode::GtEq        => ThreadedOp::GtEq,

            OpCode::Not         => ThreadedOp::Not,

            OpCode::Jump            => ThreadedOp::Jump(op),
            OpCode::JumpIfFalse     => ThreadedOp::JumpIfFalse(op),
            OpCode::JumpIfFalseOrPop=> ThreadedOp::JumpIfFalseOrPop(op),
            OpCode::JumpIfTrueOrPop => ThreadedOp::JumpIfTrueOrPop(op),
            OpCode::ForIter         => ThreadedOp::ForIter(op),
            OpCode::GetIter         => ThreadedOp::GetIter(op),

            OpCode::PopTop      => ThreadedOp::PopTop,
            OpCode::ReturnValue => ThreadedOp::ReturnValue,

            OpCode::Call            => ThreadedOp::Call(op),
            OpCode::CallPrint       => ThreadedOp::CallPrint(op),
            OpCode::CallLen         => ThreadedOp::CallLen,
            OpCode::CallAbs         => ThreadedOp::CallAbs,
            OpCode::CallStr         => ThreadedOp::CallStr,
            OpCode::CallInt         => ThreadedOp::CallInt,
            OpCode::CallFloat       => ThreadedOp::CallFloat,
            OpCode::CallBool        => ThreadedOp::CallBool,
            OpCode::CallType        => ThreadedOp::CallType,
            OpCode::CallChr         => ThreadedOp::CallChr,
            OpCode::CallOrd         => ThreadedOp::CallOrd,
            OpCode::CallSorted      => ThreadedOp::CallSorted,
            OpCode::CallList        => ThreadedOp::CallList,
            OpCode::CallTuple       => ThreadedOp::CallTuple,
            OpCode::CallEnumerate   => ThreadedOp::CallEnumerate,
            OpCode::CallIsInstance  => ThreadedOp::CallIsInstance,
            OpCode::CallRange       => ThreadedOp::CallRange(op),
            OpCode::CallRound       => ThreadedOp::CallRound(op),
            OpCode::CallMin         => ThreadedOp::CallMin(op),
            OpCode::CallMax         => ThreadedOp::CallMax(op),
            OpCode::CallSum         => ThreadedOp::CallSum(op),
            OpCode::CallZip         => ThreadedOp::CallZip(op),
            OpCode::CallDict        => ThreadedOp::CallDict(op),
            OpCode::CallSet         => ThreadedOp::CallSet(op),
            OpCode::CallInput       => ThreadedOp::CallInput,
            OpCode::MakeFunction    => ThreadedOp::MakeFunction(op),
            OpCode::MakeCoroutine   => ThreadedOp::MakeCoroutine(op),

            OpCode::GetItem         => ThreadedOp::GetItem,
            OpCode::LoadAttr        => ThreadedOp::LoadAttr(op),

            OpCode::Phi             => ThreadedOp::Phi { operand: op, rip: ip },

            OpCode::And | OpCode::Or => ThreadedOp::Unreachable,

            // All other opcodes go through Generic dispatch
            other => ThreadedOp::Generic { opcode: other, operand: op },
        });
        ip += 1;
    }
    ops
}
