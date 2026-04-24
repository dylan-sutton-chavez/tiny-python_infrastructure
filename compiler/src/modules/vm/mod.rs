// vm/mod.rs

pub mod types;
mod cache;
mod ops;
mod builtins;
mod collections;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, FastOp, Templates};
use ops::cached_binop;

use crate::modules::parser::{OpCode, SSAChunk, Value, BUILTIN_TYPES};
use alloc::{string::{String, ToString}, vec::Vec, vec, rc::Rc, format, boxed::Box};
use crate::modules::fx::FxHashMap as HashMap;
use core::cell::RefCell;

/*
VM State
    Stack, heap, iterators, yield buffer, templates and sandbox counters.
*/

pub struct VM<'a> {
    pub(crate) stack: Vec<Val>,
    pub(crate) heap: HeapPool,
    pub(crate) iter_stack: Vec<IterFrame>,
    pub(crate) yields:Vec<Val>,
    pub(crate) chunk: &'a SSAChunk,
    pub(crate) globals: HashMap<String, Val>,
    pub(crate) live_slots: Vec<Val>,
    pub output: Vec<String>,
    templates: Templates,
    budget:usize,
    depth: usize,
    max_calls: usize,
    observed_impure: Vec<bool>,
}

impl<'a> VM<'a> {
    pub fn new(chunk: &'a SSAChunk) -> Self { Self::with_limits(chunk, Limits::none()) }

    /*
    Fill Builtins
        Initializes slot vector from globals for a given name table.
    */

    fn fill_builtins(&self, names: &[String]) -> Vec<Option<Val>> {
        let mut slots = vec![None; names.len()];
        for (i, name) in names.iter().enumerate() {
            if let Some(v) = self.globals.get(name) {
                slots[i] = Some(*v);
            }
        }
        slots
    }

    /*
    Budget-Checked Jump
        Decrements op budget, validates target, returns new ip.
    */
    
    #[inline]
    fn checked_jump(&mut self, target: usize, limit: usize) -> Result<usize, VmErr> {
        if self.budget == 0 { return Err(cold_budget()); }
        self.budget -= 1;
        if target > limit { return Err(VmErr::Runtime("jump target out of bounds")); }
        Ok(target)
    }

    // Splits a heap string into individual character Val items.
    pub(crate) fn str_to_char_vals(&mut self, s: &str) -> Result<Vec<Val>, VmErr> {
        s.chars().map(|c| self.heap.alloc(HeapObj::Str(c.to_string()))).collect()
    }

    /*
    Iterator Frame Construction
        Converts a heap object to the appropriate IterFrame for ForIter dispatch.
    */
    
    fn make_iter_frame(&mut self, obj: Val) -> Result<IterFrame, VmErr> {
        if !obj.is_heap() { return Err(VmErr::Type("object is not iterable")); }
        Ok(match self.heap.get(obj) {
            HeapObj::Range(s, e, st) => IterFrame::Range { cur: *s, end: *e, step: *st },
            HeapObj::List(v) => IterFrame::Seq { items: v.borrow().clone(), idx: 0 },
            HeapObj::Tuple(v) => IterFrame::Seq { items: v.clone(), idx: 0 },
            HeapObj::Dict(p) => IterFrame::Seq { items: p.borrow().keys().collect(), idx: 0 },
            HeapObj::Set(s) => {
                let mut items: Vec<Val> = s.borrow().iter().cloned().collect();
                items.sort_by(|a, b| {
                    match (a.is_int() || a.is_float(), b.is_int() || b.is_float()) {
                        (true, true) => {
                            let fa = if a.is_int() { a.as_int() as f64 } else { a.as_float() };
                            let fb = if b.is_int() { b.as_int() as f64 } else { b.as_float() };
                            fa.partial_cmp(&fb).unwrap_or(core::cmp::Ordering::Equal)
                        }
                        (true, false)  => core::cmp::Ordering::Less,
                        (false, true)  => core::cmp::Ordering::Greater,
                        (false, false) => self.repr(*a).cmp(&self.repr(*b)),
                    }
                });
                IterFrame::Seq { items, idx: 0 }
            },
            HeapObj::Str(s) => {
                let s = s.clone();
                let items = self.str_to_char_vals(&s)?;
                IterFrame::Seq { items, idx: 0 }
            },
            _ => return Err(VmErr::Type("object is not iterable")),
        })
    }

    /*
    Sequence Unpack
        Destructures list, tuple, or string into exactly `expected` stack values.
    */
    
    fn exec_unpack_seq(&mut self, expected: usize) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(VmErr::Type("cannot unpack non-sequence")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v)  => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            HeapObj::Str(s) => {
                let s = s.clone();
                let out = self.str_to_char_vals(&s)?;
                if out.len() != expected {
                    return Err(VmErr::Value("not enough values to unpack"));
                }
                out
            },
            _ => return Err(VmErr::Type("cannot unpack non-sequence")),
        };
        if items.len() != expected {
            return Err(VmErr::Value("not enough values to unpack"));
        }
        for item in items.into_iter().rev() { self.push(item); }
        Ok(())
    }

    /*
    SSA Phi Propagation
        Merges two SSA branches into target slot and back-propagates through prev_slots chain.
    */
    
    fn exec_phi(op: u16, rip: usize, phi_map: &[usize], slots: &mut [Option<Val>], prev_slots: &[Option<u16>], phi_sources: &[(u16, u16)]) {
        let target = op as usize;
        let (ia, ib) = phi_sources[phi_map[rip]];
        let val = slots[ia as usize].or(slots[ib as usize]).unwrap_or(Val::none());
        slots[target] = Some(val);

        let mut cur = target;
        let mut guard = prev_slots.len();
        while guard > 0 {
            guard -= 1;
            match prev_slots.get(cur).and_then(|p| *p) {
                Some(prev) if (prev as usize) != cur => {
                    slots[prev as usize] = Some(val);
                    cur = prev as usize;
                }
                _ => break,
            }
        }
    }

    pub fn with_limits(chunk: &'a SSAChunk, limits: Limits) -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            iter_stack: Vec::with_capacity(16),
            yields: Vec::new(),
            chunk,
            heap: HeapPool::new(limits.heap),
            globals: HashMap::default(),
            live_slots: Vec::new(),
            templates: Templates::new(),
            budget: limits.ops,
            depth: 0,
            max_calls: limits.calls,
            output: Vec::new(),
            observed_impure: Vec::new(),
        };
        for &name in BUILTIN_TYPES {
            if let Ok(type_obj) = vm.heap.alloc(HeapObj::Type(name.to_string())) {
                vm.globals.insert(name.to_string(), type_obj);
                vm.globals.insert(format!("{}_0", name), type_obj);
            }
        }
        vm
    }

    pub fn run(&mut self) -> Result<Val, VmErr> {
        let mut slots = self.fill_builtins(&self.chunk.names);
        self.exec(self.chunk, &mut slots)
    }

    // Marks all reachable values from stack, globals, iterators and slots, then sweeps.
    fn collect(&mut self, current_slots: &[Option<Val>]) {
        for &v in &self.stack { self.heap.mark(v); }
        for &v in self.globals.values() { self.heap.mark(v); }
        for frame in &self.iter_stack {
            if let IterFrame::Seq { items, .. } = frame {
                for &v in items { self.heap.mark(v); }
            }
        }
        for &v in current_slots.iter().flatten() { self.heap.mark(v); }
        for &v in &self.live_slots { self.heap.mark(v); }
        self.heap.sweep();
    }

    pub fn heap_usage(&self) -> usize { self.heap.usage() }
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.templates.count(), self.chunk.instructions.len())
    }

    /*
    Stack Helpers
        Push, pop, pop2 and pop_n with underflow-safe error propagation.
    */

    #[inline] pub(crate) fn push(&mut self, v: Val) { self.stack.push(v); }

    #[inline] pub(crate) fn pop(&mut self) -> Result<Val, VmErr> {
        self.stack.pop().ok_or(VmErr::Runtime("stack underflow"))
    }
    #[inline] pub(crate) fn pop2(&mut self) -> Result<(Val, Val), VmErr> {
        let b = self.pop()?; let a = self.pop()?; Ok((a, b))
    }
    #[inline] pub(crate) fn pop_n(&mut self, n: usize) -> Result<Vec<Val>, VmErr> {
    let at = self.stack.len().checked_sub(n)
        .ok_or(VmErr::Runtime("stack underflow"))?;
        Ok(self.stack.split_off(at))
    }

    /*
    Const Conversion
        Converts a parser-level Value into a runtime Val, allocating heap for strings.
    */

    pub(crate) fn val_from(&mut self, v: &Value) -> Result<Val, VmErr> {
        Ok(match v {
            Value::Int(i) => {
                if *i >= Val::INT_MIN && *i <= Val::INT_MAX {
                    Val::int(*i)
                } else {
                    self.heap.alloc(HeapObj::BigInt(BigInt::from_i64(*i)))?
                }
            }
            Value::BigInt(s) => self.heap.alloc(HeapObj::BigInt(BigInt::from_decimal(s)))?,
            Value::Float(f) => Val::float(*f),
            Value::Bool(b) => Val::bool(*b),
            Value::None => Val::none(),
            Value::Str(s) => self.heap.alloc(HeapObj::Str(s.clone()))?,
        })
    }

    /*
    Fast-Path Execution
        Peeks stack without popping; returns false with stack untouched to trigger deopt.
    */

    #[inline]
    fn exec_fast(&mut self, fast: FastOp) -> Result<bool, VmErr> {
        let len = self.stack.len();
        if len < 2 { return Ok(false); }
        let a = self.stack[len - 2];
        let b = self.stack[len - 1];
        let result = match fast {
            FastOp::AddFloat if a.is_float() && b.is_float() => Val::float(a.as_float() + b.as_float()),
            FastOp::AddInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 + b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { return Ok(false); }
            }
            FastOp::SubInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 - b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { return Ok(false); }
            }
            FastOp::MulInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 * b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { return Ok(false); }
            }
            FastOp::MulFloat if a.is_float() && b.is_float() => Val::float(a.as_float() * b.as_float()),
            FastOp::LtInt if a.is_int() && b.is_int() => Val::bool(a.as_int() < b.as_int()),
            FastOp::LtFloat if a.is_float() && b.is_float() => Val::bool(a.as_float() < b.as_float()),
            FastOp::EqInt if a.is_int() && b.is_int() => Val::bool(a.as_int() == b.as_int()),
            FastOp::AddStr | FastOp::EqStr if a.is_heap() && b.is_heap() => {
                let (sa, sb) = match (self.heap.get(a), self.heap.get(b)) {
                    (HeapObj::Str(x), HeapObj::Str(y)) => (x.clone(), y.clone()),
                    _ => return Ok(false),
                };
                match fast {
                    FastOp::AddStr => self.heap.alloc(HeapObj::Str(format!("{}{}", sa, sb)))?,
                    _ => Val::bool(sa == sb),
                }
            }
            _ => return Ok(false),
        };
        // Replace both operands with computed result.
        self.stack.truncate(len - 2);
        self.push(result);
        Ok(true)
    }

    /*
    Main Dispatch Loop
        Fetches instructions by IP, routes each opcode to its handler arm.
    */

    pub(crate) fn exec(&mut self, chunk: &SSAChunk, slots: &mut [Option<Val>]) -> Result<Val, VmErr> {
        let slots_base = self.live_slots.len();
        let n = chunk.instructions.len();

        let mut cache = Box::new(OpcodeCache::new(n));

        let mut ip = 0usize;

        let prev_slots = &chunk.prev_slots;

        loop {
            if ip >= n { return Ok(Val::none()); }

            if let Some(fast) = cache.get_fast(ip) {
                ip += 1;
                if self.exec_fast(fast)? { continue; }
                cache.invalidate(ip - 1); ip -= 1;
            }

            if ip >= n {
                return Err(VmErr::Runtime("instruction pointer out of bounds"));
            }

            let ins = &chunk.instructions[ip];
            let op  = ins.operand;
            let rip = ip;
            ip += 1;

            match ins.opcode {

                // Loads
                OpCode::LoadConst => { let v = self.val_from(&chunk.constants[op as usize])?; self.push(v); }
                OpCode::LoadName => { let slot = op as usize; self.push(slots[slot].ok_or_else(|| VmErr::Name(chunk.names[slot].clone()))?); }
                OpCode::StoreName => {
                    let v = self.pop()?;
                    let slot = op as usize;
                    slots[slot] = Some(v);

                    let mut cur = slot;
                    let mut guard = prev_slots.len();
                    while guard > 0 {
                        guard -= 1;
                        match prev_slots.get(cur).and_then(|p| *p) {
                            Some(prev) if (prev as usize) != cur => {
                                slots[prev as usize] = Some(v);
                                cur = prev as usize;
                            }
                            _ => break,
                        }
                    }

                    if self.heap.needs_gc() {
                        self.collect(slots);
                    }
                }
                OpCode::LoadTrue => self.push(Val::bool(true)),
                OpCode::LoadFalse => self.push(Val::bool(false)),
                OpCode::LoadNone => self.push(Val::none()),
                OpCode::LoadEllipsis => { let v = self.heap.alloc(HeapObj::Str("...".to_string()))?; self.push(v); }

                // Arithmetic (cached)
                OpCode::Add => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache); let v = self.add_vals(a, b)?; self.push(v); }
                OpCode::Sub => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache); let v = self.sub_vals(a, b)?; self.push(v); }
                OpCode::Mul => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache); let v = self.mul_vals(a, b)?; self.push(v); }
                OpCode::Div => { let (a, b) = self.pop2()?; let v = self.div_vals(a, b)?; self.push(v); }
                
                OpCode::Mod => {
                    let (a, b) = self.pop2()?;
                    if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
                        let (_, r) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
                        let v = self.bigint_to_val(r)?;
                        self.push(v);
                    } else {
                        return Err(VmErr::Type("% requires integer operands"));
                    }
                }
                OpCode::Pow => {
                    let (a, b) = self.pop2()?;
                    if let Some(ba) = self.to_bigint(a)
                        && b.is_int() {
                            let exp = b.as_int();
                            if exp >= 0 {
                                let result = ba.pow_u32(exp as u32);
                                let v = self.bigint_to_val(result)?;
                                self.push(v);
                                continue;
                            }
                            self.push(Val::float(fpowi(ba.to_f64(), exp as i32)));
                            continue;
                    }
                    let fa = if a.is_int() { a.as_int() as f64 } else if a.is_float() { a.as_float() }
                            else { return Err(VmErr::Type("** requires numeric operands")); };
                    let fb = if b.is_int() { b.as_int() as f64 } else if b.is_float() { b.as_float() }
                            else { return Err(VmErr::Type("** requires numeric operands")); };
                    self.push(Val::float(fpowf(fa, fb)));
                }
                OpCode::FloorDiv => {
                    let (a, b) = self.pop2()?;
                    if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
                        let (q, _) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
                        let v = self.bigint_to_val(q)?;
                        self.push(v);
                    } else {
                        return Err(VmErr::Type("// requires integer operands"));
                    }
                }
                OpCode::Minus => {
                    let v = self.pop()?;
                    if v.is_int() {
                        let pushed = self.i128_to_val(-(v.as_int() as i128))?;
                        self.push(pushed);
                    } else if v.is_float() {
                        self.push(Val::float(-v.as_float()));
                    } else if v.is_heap() {
                        if let HeapObj::BigInt(b) = self.heap.get(v) {
                            let neg = b.neg();
                            let pushed = self.bigint_to_val(neg)?;
                            self.push(pushed);
                        } else {
                            return Err(VmErr::Type("unary - requires a number"));
                        }
                    } else {
                        return Err(VmErr::Type("unary - requires a number"));
                    }
                }

                // Bitwise

                OpCode::BitAnd => {
                    let (a, b) = self.pop2()?;
                    let v = self.bitwise_op(a, b, |x, y| x & y)?;
                    self.push(v);
                }
                OpCode::BitOr => {
                    let (a, b) = self.pop2()?;
                    let v = self.bitwise_op(a, b, |x, y| x | y)?;
                    self.push(v);
                }
                OpCode::BitXor => {
                    let (a, b) = self.pop2()?;
                    let v = self.bitwise_op(a, b, |x, y| x ^ y)?;
                    self.push(v);
                }
                OpCode::BitNot => {
                    let v = self.pop()?;
                    if let Some(b) = self.to_bigint(v) {
                        // ~x == -(x+1) en complemento a dos para BigInt
                        let one = BigInt::from_i64(1);
                        let result = b.add(&one).neg();
                        let out = self.bigint_to_val(result)?;
                        self.push(out);
                    } else {
                        return Err(VmErr::Type("~ requires an integer"));
                    }
                }
                OpCode::Shl => {
                    let (a, b) = self.pop2()?;
                    if !b.is_int() { return Err(VmErr::Type("shift count must be an integer")); }
                    let shift = b.as_int();
                    if shift < 0 { return Err(VmErr::Value("negative shift count")); }
                    if let Some(ba) = self.to_bigint(a) {
                        if shift >= 512 { return Err(VmErr::Value("shift too large")); }
                        let factor = BigInt::from_i64(1).shl_u32(shift as u32);
                        let result = ba.mul(&factor);
                        let out = self.bigint_to_val(result)?;
                        self.push(out);
                    } else {
                        return Err(VmErr::Type("<< requires an integer"));
                    }
                }
                OpCode::Shr => {
                    let (a, b) = self.pop2()?;
                    if !b.is_int() { return Err(VmErr::Type("shift count must be an integer")); }
                    let shift = b.as_int();
                    if shift < 0 { return Err(VmErr::Value("negative shift count")); }
                    if a.is_int() {
                        self.push(Val::int(a.as_int() >> (shift.min(63))));
                    } else if let Some(ba) = self.to_bigint(a) {
                        let result = ba.shr_u32(shift.min(1024) as u32);
                        let out = self.bigint_to_val(result)?;
                        self.push(out);
                    } else {
                        return Err(VmErr::Type(">> requires an integer"));
                    }
                }

                // Comparison (cached)

                OpCode::Eq => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache); self.push(Val::bool(eq_vals_with_heap(a, b, &self.heap))); }
                OpCode::Lt => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache); let r = self.lt_vals(a, b)?; self.push(Val::bool(r)); }
                OpCode::NotEq => { 
                    let (a, b) = self.pop2()?; 
                    self.push(Val::bool(!eq_vals_with_heap(a, b, &self.heap))); 
                }
                OpCode::Gt => { let (a,b) = self.pop2()?; let r=self.lt_vals(b,a)?; self.push(Val::bool(r)); }
                OpCode::LtEq => { let (a,b) = self.pop2()?; let r=self.lt_vals(b,a)?; self.push(Val::bool(!r)); }
                OpCode::GtEq => { let (a,b) = self.pop2()?; let r=self.lt_vals(a,b)?; self.push(Val::bool(!r)); }

                // Logic

                OpCode::And => { let (a,b) = self.pop2()?; self.push(if self.truthy(a) { b } else { a }); }
                OpCode::Or => { let (a,b) = self.pop2()?; self.push(if self.truthy(a) { a } else { b }); }
                OpCode::Not => { let v = self.pop()?; self.push(Val::bool(!self.truthy(v))); }

                // Identity / membership

                OpCode::In => { let (a,b) = self.pop2()?; self.push(Val::bool( self.contains(b, a))); }
                OpCode::NotIn => { let (a,b) = self.pop2()?; self.push(Val::bool(!self.contains(b, a))); }
                OpCode::Is => { let (a,b) = self.pop2()?; self.push(Val::bool(a.0 == b.0)); }
                OpCode::IsNot => { let (a,b) = self.pop2()?; self.push(Val::bool(a.0 != b.0)); }

                // Control flow 

                OpCode::JumpIfFalse => {
                    let v = self.pop()?;
                    if !self.truthy(v) { ip = self.checked_jump(op as usize, n)?; }
                }
                OpCode::Jump => { ip = self.checked_jump(op as usize, n)?; }
                OpCode::PopTop => { self.pop()?; }
                OpCode::Dup2 => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a); self.push(b);
                    self.push(a); self.push(b);
                }
                OpCode::ReturnValue => {
                    let result = if self.stack.is_empty() { Val::none() } else { self.pop()? };
                    self.live_slots.truncate(slots_base);
                    return Ok(result);
                }

                // Yield

                OpCode::Yield => {
                    let v = self.pop()?;
                    self.yields.push(v);
                    self.push(Val::none());
                }

                // Collections (delegated)

                OpCode::BuildList  => { let v = self.pop_n(op as usize)?; let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(v))))?; self.push(val); }
                OpCode::BuildTuple => { let v = self.pop_n(op as usize)?; let val = self.heap.alloc(HeapObj::Tuple(v))?; self.push(val); }
                OpCode::BuildDict => {
                    let flat = self.pop_n(op as usize * 2)?;
                    let dm = DictMap::from_pairs(flat.chunks(2).map(|c| (c[0], c[1])).collect());
                    let val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?; self.push(val);
                }
                OpCode::BuildString => {
                    let parts = self.pop_n(op as usize)?;
                    let s: String = parts.iter().map(|v| self.display(*v)).collect();
                    let val = self.heap.alloc(HeapObj::Str(s))?; self.push(val);
                }
                OpCode::BuildSet => { self.build_set(op)?; }
                OpCode::BuildSlice => { self.build_slice(op)?; }
                OpCode::GetItem => { self.get_item()?; }
                OpCode::StoreItem => { self.mark_impure(); self.store_item()?; }
                OpCode::UnpackSequence => { self.exec_unpack_seq(op as usize)?; }
                OpCode::UnpackEx => { self.unpack_ex(op)?; }
                OpCode::FormatValue => {
                    if op == 1 { self.pop()?; }
                    let v = self.pop()?; let s = self.display(v);
                    let val = self.heap.alloc(HeapObj::Str(s))?; self.push(val);
                }

                // Iterators

                OpCode::GetIter => {
                    let obj = self.pop()?;
                    let frame = self.make_iter_frame(obj)?;
                    self.iter_stack.push(frame);
                }
                OpCode::ForIter => {
                    if self.budget == 0 { return Err(cold_budget()); }
                    self.budget -= 1;
                    if self.heap.needs_gc() { self.collect(slots); }
                    match self.iter_stack.last_mut().and_then(|f| f.next_item()) {
                        Some(item) => self.push(item),
                        None => {
                            self.iter_stack.pop();
                            if op as usize > n { return Err(VmErr::Runtime("jump target out of bounds")); }
                            ip = op as usize;
                        }
                    }
                }

                // Comprehension append

                OpCode::ListAppend => {
                    let v = self.pop()?;
                    let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                    if !acc.is_heap() { return Err(VmErr::Runtime("list accumulator corrupted")); }
                    match self.heap.get(acc) {
                        HeapObj::List(rc) => rc.borrow_mut().push(v),
                        _ => return Err(VmErr::Runtime("list accumulator corrupted")),
                    }
                }
                OpCode::SetAdd => {
                    let v = self.pop()?;
                    let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                    if !acc.is_heap() { return Err(VmErr::Runtime("set accumulator corrupted")); }
                    let already = match self.heap.get(acc) {
                        HeapObj::Set(rc) => rc.borrow().iter().any(|&x| eq_vals_with_heap(x, v, &self.heap)),
                        _ => return Err(VmErr::Runtime("set accumulator corrupted")),
                    };
                    if !already && let HeapObj::Set(rc) = self.heap.get(acc) { rc.borrow_mut().insert(v); }
                }
                OpCode::MapAdd => {
                    let value = self.pop()?;
                    let key = self.pop()?;
                    let acc = *self.stack.last().ok_or(VmErr::Runtime("stack underflow"))?;
                    if !acc.is_heap() { return Err(VmErr::Runtime("dict accumulator corrupted")); }
                    match self.heap.get(acc) {
                        HeapObj::Dict(rc) => { rc.borrow_mut().insert(key, value); }
                        _ => return Err(VmErr::Runtime("dict accumulator corrupted")),
                    }
                }

                // SSA Phi

                OpCode::Phi => { Self::exec_phi(op, rip, &chunk.phi_map, slots, prev_slots, &chunk.phi_sources); }

                // Functions

                OpCode::MakeFunction | OpCode::MakeCoroutine => {
                    let n_defaults = self.chunk.functions[op as usize].2 as usize;
                    let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };
                    let val = self.heap.alloc(HeapObj::Func(op as usize, defaults))?;
                    self.push(val);
                }
                OpCode::Call => {
                    let raw = op as usize;
                    let num_kw = (raw >> 8) & 0xFF;
                    let num_pos = raw & 0xFF;
                    let total_items = num_pos + 2 * num_kw;

                    if self.depth >= self.max_calls { return Err(cold_depth()); }

                    let mut stack_items: Vec<Val> = (0..total_items).map(|_| self.pop()).collect::<Result<_,_>>()?;
                    stack_items.reverse();

                    let kw_flat: Vec<Val> = stack_items.split_off(num_pos);
                    let positional = stack_items;

                    let callee = self.pop()?;
                    if !callee.is_heap() { return Err(VmErr::Type("object is not callable")); }
                    let (fi, captured_defaults) = match self.heap.get(callee) {
                        HeapObj::Func(i, d) => (*i, d.clone()),
                        _ => return Err(VmErr::Type("object is not callable")),
                    };

                    if num_kw == 0
                        && let Some(cached) = self.templates.lookup(fi, &positional, &self.heap) {
                            self.push(cached); continue;
                        }

                    self.depth += 1;
                    let (params, body, _defaults, name_idx) = &self.chunk.functions[fi];
                    let mut fn_slots = self.fill_builtins(&body.names);
                    let mut body_map: HashMap<&str, usize> =
                        HashMap::with_capacity_and_hasher(body.names.len(), Default::default());
                    for (i, n) in body.names.iter().enumerate() { body_map.insert(n.as_str(), i); }

                    for (i, param) in params.iter().enumerate() {
                        if i >= positional.len() { break; }
                        let pname = format!("{}_0", param.trim_start_matches('*'));
                        if let Some(&s) = body_map.get(pname.as_str()) {
                            fn_slots[s] = Some(positional[i]);
                        }
                    }

                    for pair in kw_flat.chunks_exact(2) {
                        let name_val = pair[0];
                        let value = pair[1];
                        let key = match self.heap.get(name_val) {
                            HeapObj::Str(s) => s.clone(),
                            _ => return Err(VmErr::Runtime("malformed kwarg on stack")),
                        };
                        if params.iter().any(|p| p.trim_start_matches('*') == key.as_str()) {
                            let pname = format!("{}_0", key);
                            if let Some(&s) = body_map.get(pname.as_str()) {
                                fn_slots[s] = Some(value);
                            }
                        }
                    }

                    if !captured_defaults.is_empty() {
                        let n_params = params.len();
                        let n_defaults = captured_defaults.len();
                        let offset = n_params.saturating_sub(n_defaults);
                        for (di, &dv) in captured_defaults.iter().enumerate() {
                            if let Some(param) = params.get(offset + di) {
                                let pname = format!("{}_0", param.trim_start_matches('*'));
                                if let Some(&s) = body_map.get(pname.as_str())
                                    && fn_slots[s].is_none()
                                {
                                    fn_slots[s] = Some(dv);
                                }
                            }
                        }
                    }

                    for (si, sv) in slots.iter().enumerate() {
                        if let Some(v) = sv
                            && let Some(&bs) = body_map.get(chunk.names[si].as_str())
                            && fn_slots[bs].is_none()
                        {
                            fn_slots[bs] = Some(*v);
                        }
                    }
                    
                    // Inject callee into body slots so the function can call itself by name
                    let name_idx = *name_idx;
                    if name_idx != u16::MAX {
                        let raw = &self.chunk.names[name_idx as usize];
                        let base = raw.rfind('_').filter(|&p| raw[p+1..].parse::<u32>().is_ok()).map(|p| &raw[..p]).unwrap_or(raw.as_str());
                        let versioned = format!("{}_0", base);
                        if let Some(&slot) = body_map.get(versioned.as_str()) && fn_slots[slot].is_none() {
                            fn_slots[slot] = Some(callee);
                        }
                    }

                    let yields_before = self.yields.len();
                    let snap = self.live_slots.len();
                    self.live_slots.extend(slots.iter().flatten().copied());

                    self.observed_impure.push(false);
                    let result = self.exec(body, &mut fn_slots)?;
                    let callee_impure = self.observed_impure.pop().unwrap_or(true);
                    if callee_impure { self.mark_impure(); }

                    self.live_slots.truncate(snap);
                    self.depth -= 1;

                    if self.yields.len() > yields_before {
                        let fn_yields = self.yields.split_off(yields_before);
                        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(fn_yields))))?;
                        self.push(val);
                    } else {
                        if num_kw == 0 && body.is_pure && !callee_impure {
                            self.templates.record(fi, &positional, result, &self.heap);
                        }
                        self.push(result);
                    }
                }

                // Builtins (delegated)

                OpCode::CallPrint => { self.mark_impure(); self.call_print(op)?; }
                OpCode::CallLen => { self.call_len()?; }
                OpCode::CallAbs => { self.call_abs()?; }
                OpCode::CallStr => { self.call_str()?; }
                OpCode::CallInt => { self.call_int()?; }
                OpCode::CallFloat => { self.call_float()?; }
                OpCode::CallBool => { self.call_bool()?; }
                OpCode::CallType => { self.call_type()?; }
                OpCode::CallChr => { self.call_chr()?; }
                OpCode::CallOrd => { self.call_ord()?; }
                OpCode::CallRange => { self.call_range(op)?; }
                OpCode::CallRound => { self.call_round(op)?; }
                OpCode::CallMin => { self.call_min(op)?; }
                OpCode::CallMax => { self.call_max(op)?; }
                OpCode::CallSum => { self.call_sum(op)?; }
                OpCode::CallSorted => { self.call_sorted()?; }
                OpCode::CallList => { self.call_list()?; }
                OpCode::CallTuple => { self.call_tuple()?; }
                OpCode::CallEnumerate => { self.call_enumerate()?; }
                OpCode::CallZip => { self.call_zip(op)?; }
                OpCode::CallIsInstance => { self.call_isinstance()?; }
                OpCode::CallInput => { self.mark_impure(); self.call_input()?; }
                OpCode::CallDict => { self.call_dict(op)?; }
                OpCode::CallSet => { self.call_set(op)?; }

                // Implemented stubs

                OpCode::Assert => { let v = self.pop()?; if !self.truthy(v) { return Err(VmErr::Runtime("AssertionError")); } }
                OpCode::Del => { let slot = op as usize; if slot < slots.len() { slots[slot] = None; } }

                // No-op stubs (safe for sandbox/WASM)

                OpCode::Global | OpCode::Nonlocal => { self.mark_impure(); }
                OpCode::TypeAlias => { self.pop()?; }
                OpCode::Import => { self.mark_impure(); self.push(Val::none()); }
                OpCode::ImportFrom => { self.mark_impure(); self.pop()?; self.push(Val::none()); }
                OpCode::SetupExcept | OpCode::PopExcept => {}
                OpCode::Raise | OpCode::RaiseFrom => { self.mark_impure(); return Err(VmErr::Runtime("exception raised")); }
                OpCode::SetupWith | OpCode::ExitWith => { return Err(VmErr::Runtime("with/as not yet supported")); }
                OpCode::Await | OpCode::YieldFrom => {}
                OpCode::UnpackArgs => { return Err(VmErr::Runtime("*args/**kwargs not yet supported")); }
                OpCode::MakeClass => { return Err(VmErr::Runtime("classes not yet supported")); }
                OpCode::LoadAttr | OpCode::StoreAttr => { return Err(VmErr::Runtime("attribute access not yet supported")); }
            }
        }
    }
}