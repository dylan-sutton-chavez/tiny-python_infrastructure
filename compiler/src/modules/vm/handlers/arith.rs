// vm/handlers/arith.rs

use super::*;
use cache::OpcodeCache;
use ops::cached_binop;

impl<'a> VM<'a> {
    /*
    Arithmetic
        Add/Sub/Mul/Div con cache, Mod/Pow/FloorDiv con BigInt, Minus unario.
    */

    pub(crate) fn handle_arith(&mut self, op: OpCode, rip: usize, cache: &mut OpcodeCache) -> Result<(), VmErr> {
        if op == OpCode::Minus {
            return self.exec_neg();
        }

        let (a, b) = self.pop2()?;
        // Register-based FastOps (add/sub/mul) are cached; the rest not.
        if matches!(op, OpCode::Add | OpCode::Sub | OpCode::Mul) {
            cached_binop!(self.heap, rip, &op, a, b, cache);
        }

        let result = match op {
            OpCode::Add => self.add_vals(a, b)?,
            OpCode::Sub => self.sub_vals(a, b)?,
            OpCode::Mul => self.mul_vals(a, b)?,
            OpCode::Div => self.div_vals(a, b)?,
            OpCode::Mod => self.exec_mod(a, b)?,
            OpCode::Pow => self.exec_pow(a, b)?,
            OpCode::FloorDiv => self.exec_floordiv(a, b)?,
            _ => unreachable!("non-arith opcode in handle_arith"),
        };
        self.push(result);
        Ok(())
    }

    fn exec_neg(&mut self) -> Result<(), VmErr> {
        let v = self.pop()?;
        let result = if v.is_int() {
            self.i128_to_val(-(v.as_int() as i128))?
        } else if v.is_float() {
            Val::float(-v.as_float())
        } else if v.is_heap() {
            match self.heap.get(v) {
                HeapObj::BigInt(b) => { let n = b.neg(); self.bigint_to_val(n)? }
                _ => return Err(VmErr::Type("unary - requires a number")),
            }
        } else {
            return Err(VmErr::Type("unary - requires a number"));
        };
        self.push(result);
        Ok(())
    }

    fn exec_mod(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b))
            else { return Err(VmErr::Type("% requires integer operands")); };
        let (_, r) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
        self.bigint_to_val(r)
    }

    fn exec_floordiv(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b))
            else { return Err(VmErr::Type("// requires integer operands")); };
        let (q, _) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
        self.bigint_to_val(q)
    }

    fn exec_pow(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if let Some(ba) = self.to_bigint(a) && b.is_int() {
            let exp = b.as_int();
            if exp >= 0 {
                if exp > u32::MAX as i64 {
                    return Err(VmErr::Value("pow() exponent too large"));
                }
                return self.bigint_to_val(ba.pow_u32(exp as u32));
            }
            return Ok(Val::float(fpowi(ba.to_f64(), exp as i32)));
        }
        let to_f = |v: Val| -> Result<f64, VmErr> {
            if v.is_int() { Ok(v.as_int() as f64) }
            else if v.is_float() { Ok(v.as_float()) }
            else { Err(VmErr::Type("** requires numeric operands")) }
        };
        Ok(Val::float(fpowf(to_f(a)?, to_f(b)?)))
    }

    /*
    Bitwise
        AND/OR/XOR vía closure, NOT unario, SHL/SHR con BigInt.
    */

    pub(crate) fn handle_bitwise(&mut self, op: OpCode) -> Result<(), VmErr> {
        if op == OpCode::BitNot {
            let v = self.pop()?;
            let b = self.to_bigint(v).ok_or(VmErr::Type("~ requires an integer"))?;
            // ~x == -(x+1) en complemento a dos
            let one = BigInt::from_i64(1);
            let result = b.add(&one).neg();
            let out = self.bigint_to_val(result)?;
            self.push(out);
            return Ok(());
        }

        let (a, b) = self.pop2()?;
        let result = match op {
            OpCode::BitAnd => self.bitwise_op(a, b, |x, y| x & y)?,
            OpCode::BitOr => self.bitwise_op(a, b, |x, y| x | y)?,
            OpCode::BitXor => self.bitwise_op(a, b, |x, y| x ^ y)?,
            OpCode::Shl => self.exec_shl(a, b)?,
            OpCode::Shr => self.exec_shr(a, b)?,
            _ => unreachable!("non-bitwise opcode in handle_bitwise"),
        };
        self.push(result);
        Ok(())
    }

    fn exec_shl(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if !b.is_int() { return Err(VmErr::Type("shift count must be an integer")); }
        let shift = b.as_int();
        if shift < 0 { return Err(VmErr::Value("negative shift count")); }
        let ba = self.to_bigint(a).ok_or(VmErr::Type("<< requires an integer"))?;
        if shift >= 512 { return Err(VmErr::Value("shift too large")); }
        let factor = BigInt::from_i64(1).shl_u32(shift as u32);
        self.bigint_to_val(ba.mul(&factor))
    }

    fn exec_shr(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if !b.is_int() { return Err(VmErr::Type("shift count must be an integer")); }
        let shift = b.as_int();
        if shift < 0 { return Err(VmErr::Value("negative shift count")); }
        if a.is_int() {
            return Ok(Val::int(a.as_int() >> shift.min(63)));
        }
        let ba = self.to_bigint(a).ok_or(VmErr::Type(">> requires an integer"))?;
        self.bigint_to_val(ba.shr_u32(shift.min(1024) as u32))
    }

    /*
    Comparison
        Solo Eq y Lt registran en cache (las únicas con FastOp).
    */

    pub(crate) fn handle_compare(&mut self, op: OpCode, rip: usize, cache: &mut OpcodeCache) -> Result<(), VmErr> {
        let (a, b) = self.pop2()?;
        if matches!(op, OpCode::Eq | OpCode::Lt) {
            cached_binop!(self.heap, rip, &op, a, b, cache);
        }
        let result = match op {
            OpCode::Eq => eq_vals_with_heap(a, b, &self.heap),
            OpCode::NotEq => !eq_vals_with_heap(a, b, &self.heap),
            OpCode::Lt => self.lt_vals(a, b)?,
            OpCode::Gt => self.lt_vals(b, a)?,
            OpCode::LtEq => !self.lt_vals(b, a)?,
            OpCode::GtEq => !self.lt_vals(a, b)?,
            _ => unreachable!("non-compare opcode in handle_compare"),
        };
        self.push(Val::bool(result));
        Ok(())
    }

    /*
    Logic
        Short-circuit a nivel de valores ya evaluados: el parser garantiza la semántica.
    */

    pub(crate) fn handle_logic(&mut self, op: OpCode) -> Result<(), VmErr> {
        match op {
            OpCode::And => {
                let (a, b) = self.pop2()?;
                self.push(if self.truthy(a) { b } else { a });
            }
            OpCode::Or => {
                let (a, b) = self.pop2()?;
                self.push(if self.truthy(a) { a } else { b });
            }
            OpCode::Not => {
                let v = self.pop()?;
                self.push(Val::bool(!self.truthy(v)));
            }
            _ => unreachable!("non-logic opcode in handle_logic"),
        }
        Ok(())
    }

    /*
    Identity & Membership
        `is`/`is not` comparan tag inline; `in`/`not in` delegan en contains().
    */

    pub(crate) fn handle_identity(&mut self, op: OpCode) -> Result<(), VmErr> {
        let (a, b) = self.pop2()?;
        let result = match op {
            OpCode::In => self.contains(b, a),
            OpCode::NotIn => !self.contains(b, a),
            OpCode::Is => a.0 == b.0,
            OpCode::IsNot => a.0 != b.0,
            _ => unreachable!("non-identity opcode in handle_identity"),
        };
        self.push(Val::bool(result));
        Ok(())
    }
}