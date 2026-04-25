// vm/super_ops.rs

//! Tier-2 JIT: superinstruction patterns built from inline pieces.

use super::types::Val;
use crate::modules::parser::{OpCode, SSAChunk, Value};
use alloc::{vec, vec::Vec};

/* ── inline pieces (zero-cost, fully inlined into super bodies) ── */

#[inline(always)]
fn p_load(slots: &[Option<Val>], s: u16) -> Option<Val> {
    slots.get(s as usize).copied().flatten()
}

#[inline(always)]
fn p_add_int(a: Val, b: Val) -> Option<Val> {
    if !(a.is_int() && b.is_int()) { return None; }
    let r = (a.as_int() as i128) + (b.as_int() as i128);
    if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 {
        Some(Val::int(r as i64))
    } else { None }
}

#[inline(always)]
fn p_lt_int(a: Val, b: Val) -> Option<bool> {
    if a.is_int() && b.is_int() { Some(a.as_int() < b.as_int()) } else { None }
}

#[inline(always)]
fn p_store_ssa(slots: &mut [Option<Val>], prev: &[Option<u16>], s: u16, v: Val) {
    let mut cur = s as usize;
    if cur < slots.len() { slots[cur] = Some(v); }
    let mut guard = prev.len();
    while guard > 0 {
        guard -= 1;
        match prev.get(cur).and_then(|p| *p) {
            Some(p) if (p as usize) != cur => {
                cur = p as usize;
                if cur < slots.len() { slots[cur] = Some(v); }
            }
            _ => break,
        }
    }
}

/* ── def_super! macro ── */

/// Wraps a sequence of `#[inline(always)]` pieces into a single
/// `#[inline(never)] extern "C" fn`. The non-inlined boundary keeps super
/// bodies out of the hot dispatch loop's I-cache while still permitting full
/// inlining of the inner pieces.
#[macro_export]
macro_rules! def_super {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty $body:block
    ) => {
        $(#[$meta])*
        #[inline(never)]
        #[allow(improper_ctypes_definitions)]
        $vis extern "C" fn $name($($arg: $ty),*) -> $ret $body
    };
}

/* ── initial superinstruction compositions ── */

def_super! {
    /// load + const + add + store ⟶ `slot += delta` (e.g. `i += 1`).
    pub fn super_inc(
        slots: &mut [Option<Val>], prev: &[Option<u16>], slot: u16, delta: Val
    ) -> bool {
        let Some(v) = p_load(slots, slot) else { return false };
        let Some(r) = p_add_int(v, delta) else { return false };
        p_store_ssa(slots, prev, slot, r);
        true
    }
}

def_super! {
    /// load + load + lt ⟶ `a < b`. Returns 1/0/-1 (true/false/deopt).
    pub fn super_lt(slots: &[Option<Val>], a: u16, b: u16) -> i8 {
        let (Some(av), Some(bv)) = (p_load(slots, a), p_load(slots, b)) else { return -1 };
        match p_lt_int(av, bv) { Some(true) => 1, Some(false) => 0, None => -1 }
    }
}

def_super! {
    /// Loop-body folded: inc + lt for the canonical `i += k; i < n` header.
    pub fn super_loop_guard(
        slots: &mut [Option<Val>], prev: &[Option<u16>],
        slot: u16, delta: Val, limit: u16
    ) -> i8 {
        let Some(cur)  = p_load(slots, slot)  else { return -1 };
        let Some(next) = p_add_int(cur, delta) else { return -1 };
        p_store_ssa(slots, prev, slot, next);
        let Some(lim)  = p_load(slots, limit) else { return -1 };
        match p_lt_int(next, lim) { Some(true) => 1, Some(false) => 0, None => -1 }
    }
}

/* ── pattern catalog and detection ── */

#[derive(Debug, Clone, Copy)]
pub enum SuperOp {
    Inc       { slot: u16, delta: Val, len: u16 },
    Lt        { a: u16, b: u16, len: u16 },
    LoopGuard { slot: u16, delta: Val, limit: u16, jump_target: u16, len: u16 },
}

/// One-shot scan at chunk finalization. `Some(_)` only at pattern starts;
/// later positions covered by a longer pattern remain `None`.
pub fn detect(chunk: &SSAChunk) -> Vec<Option<SuperOp>> {
    let ins = &chunk.instructions;
    let n = ins.len();
    let mut out = vec![None; n];

    let int_const = |idx: u16| -> Option<Val> {
        match chunk.constants.get(idx as usize)? {
            Value::Int(i) if (Val::INT_MIN..=Val::INT_MAX).contains(i) => Some(Val::int(*i)),
            _ => None,
        }
    };

    let mut i = 0;
    while i < n {
        // 8-op LoopGuard: load,const,add,store ; load,load,lt,jumpiffalse
        if i + 8 <= n
            && ins[i  ].opcode == OpCode::LoadName
            && ins[i+1].opcode == OpCode::LoadConst
            && ins[i+2].opcode == OpCode::Add
            && ins[i+3].opcode == OpCode::StoreName
            && ins[i+4].opcode == OpCode::LoadName
            && ins[i+5].opcode == OpCode::LoadName
            && ins[i+6].opcode == OpCode::Lt
            && ins[i+7].opcode == OpCode::JumpIfFalse
            && ins[i].operand == ins[i+3].operand
            && ins[i].operand == ins[i+4].operand
            && let Some(delta) = int_const(ins[i+1].operand)
        {
            out[i] = Some(SuperOp::LoopGuard {
                slot: ins[i].operand, delta,
                limit: ins[i+5].operand,
                jump_target: ins[i+7].operand,
                len: 8,
            });
            i += 8; continue;
        }
        // 4-op Inc; load,const,add,store (load slot == store slot, int const).
        if i + 4 <= n
            && ins[i  ].opcode == OpCode::LoadName
            && ins[i+1].opcode == OpCode::LoadConst
            && ins[i+2].opcode == OpCode::Add
            && ins[i+3].opcode == OpCode::StoreName
            && ins[i].operand == ins[i+3].operand
            && let Some(delta) = int_const(ins[i+1].operand)
        {
            out[i] = Some(SuperOp::Inc { slot: ins[i].operand, delta, len: 4 });
            i += 4; continue;
        }
        // 3-op Lt; load, load, lt.
        if i + 3 <= n
            && ins[i  ].opcode == OpCode::LoadName
            && ins[i+1].opcode == OpCode::LoadName
            && ins[i+2].opcode == OpCode::Lt
        {
            out[i] = Some(SuperOp::Lt { a: ins[i].operand, b: ins[i+1].operand, len: 3 });
            i += 3; continue;
        }
        i += 1;
    }
    out
}