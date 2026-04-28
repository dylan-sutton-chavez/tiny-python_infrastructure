// vm/optimizer.rs

use crate::modules::parser::{OpCode, SSAChunk, Instruction, Value};
use super::types::Val;
use alloc::{vec, vec::Vec};

pub fn constant_fold(chunk: &mut SSAChunk) {
    let n = chunk.instructions.len();
    let mut known: Vec<Option<Val>> = vec![None; chunk.names.len()];
    let mut dead = vec![false; n];

    let has_successor: Vec<bool> = {
        let mut v = vec![false; chunk.names.len()];
        for p in &chunk.prev_slots {
            if let Some(prev) = p {
                let i = *prev as usize;
                if i < v.len() { v[i] = true; }
            }
        }
        // Any slot whose base name is declared `nonlocal` in a child function can be mutated at runtime via back-propagation. Treat as volatile.
        for (_, body, _, _) in &chunk.functions {
            for base in &body.nonlocals {
                for (i, name) in chunk.names.iter().enumerate() {
                    if name.rfind('_').is_some_and(|p| &name[..p] == base.as_str()) && i < v.len() {
                        v[i] = true;
                    }
                }
            }
        }
        v
    };

    for ip in 0..n {
        let ins = chunk.instructions[ip];
        match ins.opcode {

            OpCode::LoadConst => {
                if let Some(next) = chunk.instructions.get(ip + 1)
                    && next.opcode == OpCode::StoreName
                    && let Some(v) = const_to_val(&chunk.constants, ins.operand)
                {
                    let slot = next.operand as usize;
                    if slot < known.len() { known[slot] = Some(v); }
                }
            }

            OpCode::LoadName => {
                let slot = ins.operand as usize;
                if has_successor.get(slot).copied().unwrap_or(true) {
                    // El slot puede reasignarse: no plegar.
                } else if let Some(Some(v)) = known.get(slot).copied()
                    && let Some(idx) = find_or_push_const(chunk, v)
                {
                    chunk.instructions[ip] = Instruction {
                        opcode: OpCode::LoadConst,
                        operand: idx,
                    };
                }
            }

            OpCode::StoreName => {
                let prev_ip = ip.wrapping_sub(1);
                if let Some(prev) = chunk.instructions.get(prev_ip)
                    && prev.opcode == OpCode::LoadConst
                    && let Some(v) = const_to_val(&chunk.constants, prev.operand)
                {
                    let slot = ins.operand as usize;
                    if slot < known.len() { known[slot] = Some(v); }
                }
            }

            OpCode::Add | OpCode::Sub | OpCode::Mul => {
                let prev2 = ip.wrapping_sub(2);
                let prev1 = ip.wrapping_sub(1);
                if let (Some(p2), Some(p1)) = (chunk.instructions.get(prev2), chunk.instructions.get(prev1))
                    && p2.opcode == OpCode::LoadConst
                    && p1.opcode == OpCode::LoadConst
                    && let (Some(a), Some(b)) = (
                        const_to_val(&chunk.constants, p2.operand),
                        const_to_val(&chunk.constants, p1.operand),
                    )
                    && let Some(result) = fold_binop(ins.opcode, a, b)
                    && let Some(idx) = find_or_push_const(chunk, result)
                {
                    chunk.instructions[prev2] = Instruction { opcode: OpCode::LoadConst, operand: idx };
                    dead[prev1] = true;
                    dead[ip]    = true;
                }
            }

            _ => {}
        }
    }

    let mut idx = 0usize;
    chunk.instructions.retain(|_| {
        let keep = !dead[idx];
        idx += 1;
        keep
    });
}

fn const_to_val(constants: &[Value], idx: u16) -> Option<Val> {
    match constants.get(idx as usize)? {
        Value::Int(i) if (Val::INT_MIN..=Val::INT_MAX).contains(i) => Some(Val::int(*i)),
        Value::Float(f) => Some(Val::float(*f)),
        Value::Bool(b) => Some(Val::bool(*b)),
        _ => None,
    }
}

fn find_or_push_const(chunk: &mut SSAChunk, v: Val) -> Option<u16> {
    let target: Value = if v.is_int() {
        Value::Int(v.as_int())
    } else if v.is_float() {
        Value::Float(v.as_float())
    } else if v.is_bool() {
        Value::Bool(v.as_bool())
    } else {
        return None;
    };
    if let Some(pos) = chunk.constants.iter().position(|c| c == &target) {
        return u16::try_from(pos).ok();
    }
    let idx = chunk.constants.len();
    chunk.constants.push(target);
    u16::try_from(idx).ok()
}

fn fold_binop(op: OpCode, a: Val, b: Val) -> Option<Val> {
    if a.is_int() && b.is_int() {
        let (ai, bi) = (a.as_int() as i128, b.as_int() as i128);
        let r = match op {
            OpCode::Add => ai + bi,
            OpCode::Sub => ai - bi,
            OpCode::Mul => ai * bi,
            _ => return None,
        };
        if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 {
            return Some(Val::int(r as i64));
        }
        return None;
    }
    if a.is_float() || b.is_float() {
        let af = if a.is_int() { a.as_int() as f64 } else { a.as_float() };
        let bf = if b.is_int() { b.as_int() as f64 } else { b.as_float() };
        return Some(Val::float(match op {
            OpCode::Add => af + bf,
            OpCode::Sub => af - bf,
            OpCode::Mul => af * bf,
            _ => return None,
        }));
    }
    None
}