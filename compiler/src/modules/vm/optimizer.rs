// vm/optimizer.rs
//
// Constant folding pass over SSA bytecode chunks.
//
// Folds:
//   LoadConst a, LoadConst b, BinOp     -> LoadConst (a OP b)
//   LoadConst v, Not                    -> LoadTrue / LoadFalse
//   LoadConst v, Minus                  -> LoadConst (-v)
//
// Does NOT fold LoadName even when the value is statically known. SSA name
// loads carry information used by tier-1 IC, super-op detection, and the
// memoization layer; replacing them with constants pessimises those paths.
//
// After folding, instructions marked dead are removed and all jump operands
// are remapped to their new positions in the compacted array.

use crate::modules::parser::{OpCode, SSAChunk, Instruction, Value};
use super::types::Val;
use alloc::{vec, vec::Vec};

pub fn constant_fold(chunk: &mut SSAChunk) {
    let n = chunk.instructions.len();
    if n == 0 {
        for (_, body, _, _) in chunk.functions.iter_mut() { constant_fold(body); }
        for class_body in chunk.classes.iter_mut() { constant_fold(class_body); }
        return;
    }

    let mut dead = vec![false; n];

    for ip in 0..n {
        if dead[ip] { continue; }
        let opcode = chunk.instructions[ip].opcode;

        match opcode {
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
            | OpCode::Mod | OpCode::FloorDiv
            | OpCode::Eq  | OpCode::NotEq
            | OpCode::Lt  | OpCode::Gt | OpCode::LtEq | OpCode::GtEq
            | OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor
            | OpCode::Shl | OpCode::Shr => {
                try_fold_binop(chunk, &mut dead, ip);
            }
            OpCode::Not => try_fold_not(chunk, &mut dead, ip),
            OpCode::Minus => try_fold_neg(chunk, &mut dead, ip),
            _ => {}
        }
    }

    if dead.iter().any(|&d| d) {
        compact_with_jump_remap(chunk, &dead);
    }

    for (_, body, _, _) in chunk.functions.iter_mut() {
        constant_fold(body);
    }
    for class_body in chunk.classes.iter_mut() {
        constant_fold(class_body);
    }
}

#[inline]
fn is_jump_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Jump
        | OpCode::JumpIfFalse
        | OpCode::JumpIfFalseOrPop
        | OpCode::JumpIfTrueOrPop
        | OpCode::ForIter
        | OpCode::SetupExcept
    )
}

// Build remap[i] = new index of instruction i after compaction. Dead targets
// forward to the next live instruction; one-past-end (i == n) maps to the
// new chunk length so jumps that fall through to the end stay valid.
fn compact_with_jump_remap(chunk: &mut SSAChunk, dead: &[bool]) {
    let n = chunk.instructions.len();
    let alive_count: usize = dead.iter().filter(|&&d| !d).count();

    let mut remap: Vec<usize> = Vec::with_capacity(n + 1);
    let mut new_pos = 0usize;
    for &is_dead in dead {
        remap.push(new_pos);
        if !is_dead { new_pos += 1; }
    }
    remap.push(alive_count);

    // Forward dead targets to their next live successor (walk back-to-front
    // so each dead entry inherits the already-corrected next entry).
    for i in (0..n).rev() {
        if dead[i] {
            remap[i] = remap[i + 1];
        }
    }

    for (ip, _) in dead.iter().enumerate().take(n) {
        if dead[ip] { continue; }
        let ins = &mut chunk.instructions[ip];
        if !is_jump_op(ins.opcode) { continue; }
        let target = ins.operand as usize;
        let new_target = if target > n { target } else { remap[target] };
        if let Ok(v) = u16::try_from(new_target) {
            ins.operand = v;
        }
    }

    let mut idx = 0usize;
    chunk.instructions.retain(|_| {
        let keep = !dead[idx];
        idx += 1;
        keep
    });
}

fn write_const_load(chunk: &mut SSAChunk, pos: usize, v: Val) -> bool {
    let new_ins = if v.is_bool() {
        Instruction {
            opcode: if v.as_bool() { OpCode::LoadTrue } else { OpCode::LoadFalse },
            operand: 0,
        }
    } else if v.is_none() {
        Instruction { opcode: OpCode::LoadNone, operand: 0 }
    } else if let Some(idx) = find_or_push_const(chunk, v) {
        Instruction { opcode: OpCode::LoadConst, operand: idx }
    } else {
        return false;
    };
    chunk.instructions[pos] = new_ins;
    true
}

fn find_or_push_const(chunk: &mut SSAChunk, v: Val) -> Option<u16> {
    let target: Value = if v.is_int() {
        Value::Int(v.as_int())
    } else if v.is_float() {
        Value::Float(v.as_float())
    } else {
        return None;
    };
    if let Some(pos) = chunk.constants.iter().position(|c| c == &target) {
        return u16::try_from(pos).ok();
    }
    let idx = chunk.constants.len();
    if idx >= u16::MAX as usize { return None; }
    chunk.constants.push(target);
    u16::try_from(idx).ok()
}

fn const_to_val(constants: &[Value], idx: u16) -> Option<Val> {
    match constants.get(idx as usize)? {
        Value::Int(i) if (Val::INT_MIN..=Val::INT_MAX).contains(i) => Some(Val::int(*i)),
        Value::Float(f) => Some(Val::float(*f)),
        Value::Bool(b) => Some(Val::bool(*b)),
        _ => None,
    }
}

// The most recent live instruction strictly before `from`. Required because
// constant folding leaves gaps (the operands of an inner fold are dead), so
// a naive `from - 1` would read instructions that no longer exist.
fn prev_live(dead: &[bool], from: usize) -> Option<usize> {
    let mut i = from;
    while i > 0 {
        i -= 1;
        if !dead[i] { return Some(i); }
    }
    None
}

fn try_fold_binop(chunk: &mut SSAChunk, dead: &mut [bool], ip: usize) {
    let Some(prev1_ip) = prev_live(dead, ip) else { return };
    let Some(prev2_ip) = prev_live(dead, prev1_ip) else { return };

    let p2 = chunk.instructions[prev2_ip];
    let p1 = chunk.instructions[prev1_ip];
    if p2.opcode != OpCode::LoadConst || p1.opcode != OpCode::LoadConst { return; }

    let (Some(a), Some(b)) = (
        const_to_val(&chunk.constants, p2.operand),
        const_to_val(&chunk.constants, p1.operand),
    ) else { return };

    let opcode = chunk.instructions[ip].opcode;
    let Some(result) = fold_binop(opcode, a, b) else { return };

    if !write_const_load(chunk, prev2_ip, result) { return; }
    dead[prev1_ip] = true;
    dead[ip] = true;
}

fn try_fold_not(chunk: &mut SSAChunk, dead: &mut [bool], ip: usize) {
    let Some(prev1_ip) = prev_live(dead, ip) else { return };
    let p1 = chunk.instructions[prev1_ip];
    if p1.opcode != OpCode::LoadConst { return; }
    let Some(v) = const_to_val(&chunk.constants, p1.operand) else { return };

    let folded = if v.is_bool() {
        Some(Val::bool(!v.as_bool()))
    } else if v.is_int() {
        Some(Val::bool(v.as_int() == 0))
    } else if v.is_float() {
        Some(Val::bool(v.as_float() == 0.0))
    } else if v.is_none() {
        Some(Val::bool(true))
    } else {
        None
    };

    if let Some(r) = folded
        && write_const_load(chunk, prev1_ip, r)
    {
        dead[ip] = true;
    }
}

fn try_fold_neg(chunk: &mut SSAChunk, dead: &mut [bool], ip: usize) {
    let Some(prev1_ip) = prev_live(dead, ip) else { return };
    let p1 = chunk.instructions[prev1_ip];
    if p1.opcode != OpCode::LoadConst { return; }
    let Some(v) = const_to_val(&chunk.constants, p1.operand) else { return };

    let folded = if v.is_int() {
        let r = -(v.as_int() as i128);
        if (Val::INT_MIN as i128..=Val::INT_MAX as i128).contains(&r) {
            Some(Val::int(r as i64))
        } else { None }
    } else if v.is_float() {
        Some(Val::float(-v.as_float()))
    } else { None };

    if let Some(r) = folded
        && write_const_load(chunk, prev1_ip, r)
    {
        dead[ip] = true;
    }
}

fn fold_binop(op: OpCode, a: Val, b: Val) -> Option<Val> {
    if matches!(op, OpCode::Eq | OpCode::NotEq | OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq) {
        let (af, bf) = if a.is_int() && b.is_int() {
            (a.as_int() as f64, b.as_int() as f64)
        } else if (a.is_int() || a.is_float()) && (b.is_int() || b.is_float()) {
            let af = if a.is_int() { a.as_int() as f64 } else { a.as_float() };
            let bf = if b.is_int() { b.as_int() as f64 } else { b.as_float() };
            (af, bf)
        } else {
            return None;
        };
        return Some(Val::bool(match op {
            OpCode::Eq    => af == bf,
            OpCode::NotEq => af != bf,
            OpCode::Lt    => af <  bf,
            OpCode::Gt    => af >  bf,
            OpCode::LtEq  => af <= bf,
            OpCode::GtEq  => af >= bf,
            _ => unreachable!(),
        }));
    }

    if a.is_int() && b.is_int() {
        let (ai, bi) = (a.as_int() as i128, b.as_int() as i128);
        let r = match op {
            OpCode::Add      => ai.checked_add(bi)?,
            OpCode::Sub      => ai.checked_sub(bi)?,
            OpCode::Mul      => ai.checked_mul(bi)?,
            OpCode::Mod      => if bi == 0 { return None; } else { ai.rem_euclid(bi) },
            OpCode::FloorDiv => if bi == 0 { return None; } else { ai.div_euclid(bi) },
            OpCode::BitAnd   => ai & bi,
            OpCode::BitOr    => ai | bi,
            OpCode::BitXor   => ai ^ bi,
            OpCode::Shl      => if !(0..63).contains(&bi) { return None; } else { ai.checked_shl(bi as u32)? },
            OpCode::Shr      => if !(0..63).contains(&bi) { return None; } else { ai >> bi },
            _ => return None,
        };
        if (Val::INT_MIN as i128..=Val::INT_MAX as i128).contains(&r) {
            return Some(Val::int(r as i64));
        }
        return None;
    }

    if (a.is_int() || a.is_float()) && (b.is_int() || b.is_float()) {
        let af = if a.is_int() { a.as_int() as f64 } else { a.as_float() };
        let bf = if b.is_int() { b.as_int() as f64 } else { b.as_float() };
        return Some(Val::float(match op {
            OpCode::Add => af + bf,
            OpCode::Sub => af - bf,
            OpCode::Mul => af * bf,
            OpCode::Div => if bf == 0.0 { return None; } else { af / bf },
            _ => return None,
        }));
    }

    None
}