// vm/super_ops.rs

//! Tier-2 dispatch: superinstruction patterns and closed-form loop fusion.
//!
//! - Small supers (`Inc`, `Lt`, `LoopGuard`) collapse 3-8 ops into one
//!   `extern "C"` call. Inner pieces are `#[inline(always)]`; the boundary
//!   keeps super bodies out of the hot dispatch loop's I-cache.
//! - `RangeIncFused` recognises `for _ in range(N): counter += k` and
//!   evaluates the entire loop in O(1) via `final = initial + delta * N`.

use super::types::{Val, IterFrame};
use crate::modules::parser::{OpCode, SSAChunk, Value};
use alloc::{vec, vec::Vec};

/* ─── inline pieces (zero-cost, fully inlined into super bodies) ─── */

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

/// Mirrors `handle_store`: writes `v` to slot `s` and back-propagates through
/// the SSA `prev_slots` chain so the Phi at the join sees the new value.
#[inline(always)]
pub(crate) fn p_store_ssa(slots: &mut [Option<Val>], prev: &[Option<u16>], s: u16, v: Val) {
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

/* ─── def_super! macro ─── */

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

/* ─── small supers ─── */

def_super! {
    /// `load -> +const -> store` (e.g. `i = i + 1` across two SSA versions).
    pub fn super_inc(
        slots: &mut [Option<Val>], prev: &[Option<u16>],
        load: u16, store: u16, delta: Val,
    ) -> bool {
        let Some(v) = p_load(slots, load) else { return false };
        let Some(r) = p_add_int(v, delta) else { return false };
        p_store_ssa(slots, prev, store, r);
        true
    }
}

def_super! {
    /// `load(a) load(b) lt`. Returns 1=true, 0=false, -1=deopt.
    pub fn super_lt(slots: &[Option<Val>], a: u16, b: u16) -> i8 {
        let (Some(av), Some(bv)) = (p_load(slots, a), p_load(slots, b)) else { return -1 };
        match p_lt_int(av, bv) { Some(true) => 1, Some(false) => 0, None => -1 }
    }
}

def_super! {
    /// `i += k; i < n` while-header fusion. Returns 1=continue, 0=exit, -1=deopt.
    pub fn super_loop_guard(
        slots: &mut [Option<Val>], prev: &[Option<u16>],
        load: u16, store: u16, delta: Val, limit: u16,
    ) -> i8 {
        let Some(cur)  = p_load(slots, load)  else { return -1 };
        let Some(next) = p_add_int(cur, delta) else { return -1 };
        p_store_ssa(slots, prev, store, next);
        let Some(lim)  = p_load(slots, limit) else { return -1 };
        match p_lt_int(next, lim) { Some(true) => 1, Some(false) => 0, None => -1 }
    }
}

/* ─── closed-form loop fusion ─── */

pub enum FusedOutcome { Done, Deopt }

/// Closed-form executor for `RangeIncFused`. Evaluates the entire loop as
/// `final = initial + delta * N` in O(1). Charges 2*N to the budget so
/// sandbox accounting matches per-iteration bytecode dispatch.
#[derive(Clone, Copy)]
pub struct RangeIncOps {
    pub drop:  u16,
    pub load:  u16,
    pub store: u16,
    pub delta: Val,
}

#[inline]
pub fn run_range_inc_fused(
    slots: &mut [Option<Val>],
    prev: &[Option<u16>],
    iter: &IterFrame,
    budget: &mut usize,
    ops: RangeIncOps,
) -> FusedOutcome {
    let (cur, end, step) = match *iter {
        IterFrame::Range { cur, end, step } => (cur, end, step),
        _ => return FusedOutcome::Deopt,
    };

    let n: i64 = if step > 0 {
        if cur >= end { 0 } else { (end - cur + step - 1) / step }
    } else if cur <= end { 0 } else { (cur - end - step - 1) / -step };

    if n == 0 { return FusedOutcome::Done; }

    // 1. Validate everything before any side effect.
    let Some(initial) = p_load(slots, ops.load) else { return FusedOutcome::Deopt };
    if !initial.is_int() || !ops.delta.is_int() { return FusedOutcome::Deopt; }

    let total = (ops.delta.as_int() as i128).checked_mul(n as i128);
    let Some(total) = total else { return FusedOutcome::Deopt };
    let final_v = (initial.as_int() as i128).checked_add(total);
    let Some(final_v) = final_v else { return FusedOutcome::Deopt };
    if final_v < Val::INT_MIN as i128 || final_v > Val::INT_MAX as i128 {
        return FusedOutcome::Deopt;
    }

    let last_iter = match step.checked_mul(n - 1).and_then(|s| cur.checked_add(s)) {
        Some(v) => v,
        None => return FusedOutcome::Deopt,
    };

    // 2. Charge budget last so deopts don't corrupt accounting.
    let charge = (n as usize).saturating_mul(2);
    if *budget < charge { return FusedOutcome::Deopt; }
    *budget -= charge;

    // 3. Commit.
    p_store_ssa(slots, prev, ops.store, Val::int(final_v as i64));
    p_store_ssa(slots, prev, ops.drop, Val::int(last_iter));
    FusedOutcome::Done
}

/* ─── pattern catalog ─── */

#[derive(Debug, Clone, Copy)]
pub enum SuperOp {
    Inc       { load: u16, store: u16, delta: Val, len: u16 },
    Lt        { a: u16, b: u16, len: u16 },
    LoopGuard { load: u16, store: u16, delta: Val, limit: u16, jump_target: u16, len: u16 },
    /// Triggers at ForIter; iter on top of stack must be Range.
    /// 7-op pattern: ForIter <end>, StoreName _drop, LoadName c, LoadConst k,
    ///               Add, StoreName c', Jump <self>
    RangeIncFused {
        drop_slot:     u16,
        counter_load:  u16,
        counter_store: u16,
        delta:         Val,
        end_ip:        u16,
    },
}

/* ─── detection (one-shot scan at chunk finalization) ─── */

pub fn detect(chunk: &SSAChunk) -> Vec<Option<SuperOp>> {
    let ins = &chunk.instructions;
    let n = ins.len();
    let mut out = vec![None; n];
    let prev = &chunk.prev_slots;

    let int_const = |idx: u16| -> Option<Val> {
        match chunk.constants.get(idx as usize)? {
            Value::Int(i) if (Val::INT_MIN..=Val::INT_MAX).contains(i) => Some(Val::int(*i)),
            _ => None,
        }
    };

    // SSA-aware: `store` is the immediate successor version of `load`
    // (or, in non-SSA edge cases, the same slot).
    let same_var = |load_op: u16, store_op: u16| -> bool {
        load_op == store_op
        || prev.get(store_op as usize).and_then(|p| *p) == Some(load_op)
    };

    let mut i = 0;
    while i < n {
        // ── 7-op RangeIncFused (most aggressive; check first) ───────────
        if i + 7 <= n
            && ins[i  ].opcode == OpCode::ForIter
            && ins[i+1].opcode == OpCode::StoreName
            && ins[i+2].opcode == OpCode::LoadName
            && ins[i+3].opcode == OpCode::LoadConst
            && ins[i+4].opcode == OpCode::Add
            && ins[i+5].opcode == OpCode::StoreName
            && ins[i+6].opcode == OpCode::Jump
            && ins[i+6].operand as usize == i
            && same_var(ins[i+2].operand, ins[i+5].operand)
            && let Some(delta) = int_const(ins[i+3].operand)
        {
            out[i] = Some(SuperOp::RangeIncFused {
                drop_slot:     ins[i+1].operand,
                counter_load:  ins[i+2].operand,
                counter_store: ins[i+5].operand,
                delta,
                end_ip:        ins[i].operand,
            });
            i += 7; continue;
        }

        // ── 8-op LoopGuard ──────────────────────────────────────────────
        if i + 8 <= n
            && ins[i  ].opcode == OpCode::LoadName
            && ins[i+1].opcode == OpCode::LoadConst
            && ins[i+2].opcode == OpCode::Add
            && ins[i+3].opcode == OpCode::StoreName
            && ins[i+4].opcode == OpCode::LoadName
            && ins[i+5].opcode == OpCode::LoadName
            && ins[i+6].opcode == OpCode::Lt
            && ins[i+7].opcode == OpCode::JumpIfFalse
            && same_var(ins[i].operand, ins[i+3].operand)
            && ins[i+3].operand == ins[i+4].operand
            && let Some(delta) = int_const(ins[i+1].operand)
        {
            out[i] = Some(SuperOp::LoopGuard {
                load: ins[i].operand, store: ins[i+3].operand, delta,
                limit: ins[i+5].operand, jump_target: ins[i+7].operand, len: 8,
            });
            i += 8; continue;
        }

        // ── 4-op Inc ────────────────────────────────────────────────────
        if i + 4 <= n
            && ins[i  ].opcode == OpCode::LoadName
            && ins[i+1].opcode == OpCode::LoadConst
            && ins[i+2].opcode == OpCode::Add
            && ins[i+3].opcode == OpCode::StoreName
            && same_var(ins[i].operand, ins[i+3].operand)
            && let Some(delta) = int_const(ins[i+1].operand)
        {
            out[i] = Some(SuperOp::Inc {
                load: ins[i].operand, store: ins[i+3].operand, delta, len: 4,
            });
            i += 4; continue;
        }

        // ── 3-op Lt ─────────────────────────────────────────────────────
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