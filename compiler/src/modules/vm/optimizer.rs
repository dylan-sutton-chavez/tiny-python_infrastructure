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
        for prev in chunk.prev_slots.iter().flatten() {
            let i = *prev as usize;
            if i < v.len() { v[i] = true; }
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

            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
          | OpCode::Mod | OpCode::FloorDiv
          | OpCode::Eq  | OpCode::NotEq | OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq
          | OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor
          | OpCode::Shl | OpCode::Shr => {
                let prev2 = ip.wrapping_sub(2);
                let prev1 = ip.wrapping_sub(1);
                if dead.get(prev2).copied().unwrap_or(true) || dead.get(prev1).copied().unwrap_or(true) {
                    // Una de las dos LoadConst ya fue marcada muerta por un fold anterior.
                    // No tocar: el chunk está en estado intermedio.
                } else if let (Some(p2), Some(p1)) = (chunk.instructions.get(prev2), chunk.instructions.get(prev1))
                    && p2.opcode == OpCode::LoadConst
                    && p1.opcode == OpCode::LoadConst
                    && let (Some(a), Some(b)) = (
                        const_to_val(&chunk.constants, p2.operand),
                        const_to_val(&chunk.constants, p1.operand),
                    )
                    && let Some(result) = fold_binop(ins.opcode, a, b)
                    && write_const_load(chunk, prev2, result)
                {
                    dead[prev1] = true;
                    dead[ip]    = true;
                }
            }

            // Plegar `Not` sobre constante booleana
            OpCode::Not => {
                let prev1 = ip.wrapping_sub(1);
                if let Some(p1) = chunk.instructions.get(prev1)
                    && p1.opcode == OpCode::LoadConst
                    && let Some(v) = const_to_val(&chunk.constants, p1.operand)
                {
                    let folded = if v.is_bool() {
                        Some(Val::bool(!v.as_bool()))
                    } else if v.is_int() {
                        Some(Val::bool(v.as_int() == 0))
                    } else if v.is_none() {
                        Some(Val::bool(true))
                    } else { None };
                    if let Some(r) = folded
                        && let Some(idx) = find_or_push_const(chunk, r)
                    {
                        chunk.instructions[prev1] = Instruction { opcode: OpCode::LoadConst, operand: idx };
                        dead[ip] = true;
                    }
                }
            }

            // Plegar `Minus` (negación unaria) sobre constante
            OpCode::Minus => {
                let prev1 = ip.wrapping_sub(1);
                if let Some(p1) = chunk.instructions.get(prev1)
                    && p1.opcode == OpCode::LoadConst
                    && let Some(v) = const_to_val(&chunk.constants, p1.operand)
                {
                    let folded = if v.is_int() {
                        let r = -(v.as_int() as i128);
                        if (Val::INT_MIN as i128..=Val::INT_MAX as i128).contains(&r) {
                            Some(Val::int(r as i64))
                        } else { None }
                    } else if v.is_float() {
                        Some(Val::float(-v.as_float()))
                    } else { None };
                    if let Some(r) = folded
                        && let Some(idx) = find_or_push_const(chunk, r)
                    {
                        chunk.instructions[prev1] = Instruction { opcode: OpCode::LoadConst, operand: idx };
                        dead[ip] = true;
                    }
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

    for (_, body, _, _) in chunk.functions.iter_mut() {
        constant_fold(body);
    }
    for class_body in chunk.classes.iter_mut() {
        constant_fold(class_body);
    }
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
    } else {
        return None;  // bool y none se emiten como opcodes dedicados
    };
    if let Some(pos) = chunk.constants.iter().position(|c| c == &target) {
        return u16::try_from(pos).ok();
    }
    let idx = chunk.constants.len();
    chunk.constants.push(target);
    u16::try_from(idx).ok()
}

/// Reemplaza la instrucción en `pos` con la opcode adecuada para `v`.
/// True/False/None usan opcodes dedicados (sin entrada en el pool).
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

fn fold_binop(op: OpCode, a: Val, b: Val) -> Option<Val> {
    // Comparaciones (cualquier tipo numérico)
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

    // Aritmética entera (con detección de overflow)
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
        return None;  // overflow → no plegar, dejar al runtime promover a BigInt
    }

    // Aritmética en coma flotante
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