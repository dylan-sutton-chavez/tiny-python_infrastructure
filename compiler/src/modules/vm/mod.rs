// vm/mod.rs

pub mod types;
mod cache;
mod ops;
mod builtins;
mod collections;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::*;
use ops::cached_binop;

use crate::modules::parser::{OpCode, SSAChunk, Value, BUILTIN_TYPES};
use alloc::{string::{String, ToString}, vec::Vec, vec, rc::Rc, format, boxed::Box};
use hashbrown::HashMap;
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
    templates: Templates,
    budget:usize,
    depth: usize,
    max_calls: usize,
    pub output: Vec<String>,
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

    pub fn with_limits(chunk: &'a SSAChunk, limits: Limits) -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            iter_stack: Vec::with_capacity(16),
            yields: Vec::new(),
            chunk,
            heap: HeapPool::new(limits.heap),
            globals: HashMap::new(),
            live_slots: Vec::new(),
            templates: Templates::new(),
            budget: limits.ops,
            depth: 0,
            max_calls: limits.calls,
            output: Vec::new()
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

    fn collect(&mut self, current_slots: &[Option<Val>]) { // Marks all reachable values from stack, globals, iterators and slots, then sweeps.
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
        self.stack.pop().ok_or_else(|| VmErr::Runtime("stack underflow".into()))
    }
    #[inline] pub(crate) fn pop2(&mut self) -> Result<(Val, Val), VmErr> {
        let b = self.pop()?; let a = self.pop()?; Ok((a, b))
    }
    #[inline] pub(crate) fn pop_n(&mut self, n: usize) -> Result<Vec<Val>, VmErr> {
        let at = self.stack.len().checked_sub(n)
            .ok_or_else(|| VmErr::Runtime("stack underflow".into()))?;
        Ok(self.stack.split_off(at))
    }

    /*
    Const Conversion
        Converts a parser-level Value into a runtime Val, allocating heap for strings.
    */

    pub(crate) fn to_val(&mut self, v: &Value) -> Result<Val, VmErr> {
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
            FastOp::AddInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 + b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { Val::float(r as f64) }
            }
            FastOp::AddFloat if a.is_float() && b.is_float() => Val::float(a.as_float() + b.as_float()),
            FastOp::SubInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 - b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { Val::float(r as f64) }
            }
            FastOp::SubFloat if a.is_float() && b.is_float() => Val::float(a.as_float() - b.as_float()),
            FastOp::MulInt if a.is_int() && b.is_int() => {
                let r = a.as_int() as i128 * b.as_int() as i128;
                if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 { Val::int(r as i64) } else { Val::float(r as f64) }
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

    pub(crate) fn exec(&mut self, chunk: &SSAChunk, slots: &mut Vec<Option<Val>>) -> Result<Val, VmErr> {
        let slots_base = self.live_slots.len();
        let n = chunk.instructions.len();

        // Box per-frame caches to reduce stack frame size in debug builds
        let mut cache = Box::new(InlineCache::new(n));
        let mut adaptive = Box::new(Adaptive::new(n));
        let mut ip = 0usize;
        let mut phi_idx = 0usize;

        let prev_slots = &chunk.prev_slots; // SSA alias table: pre-computed in SSAChunk, maps each versioned slot to its predecessor.

        loop {
            if ip >= n { return Ok(Val::none()); }

            // Adaptive / inline cache fast paths
            if let Some(fast) = adaptive.get(ip) {
                ip += 1;
                if self.exec_fast(fast)? { continue; }
                adaptive.deopt(ip - 1); cache.invalidate(ip - 1); ip -= 1;
            } else if let Some(fast) = cache.get(ip) {
                ip += 1;
                if self.exec_fast(fast)? { continue; }
                cache.invalidate(ip - 1); ip -= 1;
            }

            if ip >= n {
                return Err(VmErr::Runtime("instruction pointer out of bounds".into()));
            }

            let ins = &chunk.instructions[ip];
            let op  = ins.operand;
            let rip = ip;
            ip += 1;

            match ins.opcode {

                // Loads

                OpCode::LoadConst => { let v = self.to_val(&chunk.constants[op as usize])?; self.push(v); }
                OpCode::LoadName => { let slot = op as usize; self.push(slots[slot].ok_or_else(|| VmErr::Name(chunk.names[slot].clone()))?); }
                OpCode::StoreName => {
                    let v = self.pop()?;
                    let slot = op as usize;
                    slots[slot] = Some(v);
                    if let Some(prev) = prev_slots[slot] { slots[prev as usize] = Some(v); }

                    if self.heap.needs_gc() {
                        self.collect(slots); // Garbage collector safepoint; store is the only opcode that grows the heap unboundedly
                    }
                }
                OpCode::LoadTrue => self.push(Val::bool(true)),
                OpCode::LoadFalse => self.push(Val::bool(false)),
                OpCode::LoadNone => self.push(Val::none()),
                OpCode::LoadEllipsis => { let v = self.heap.alloc(HeapObj::Str("...".into()))?; self.push(v); }

                // Arithmetic (cached)

                OpCode::Add => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache, adaptive); let v = self.add_vals(a, b)?; self.push(v); }
                OpCode::Sub => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache, adaptive); let v = self.sub_vals(a, b)?; self.push(v); }
                OpCode::Mul => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache, adaptive); let v = self.mul_vals(a, b)?; self.push(v); }
                OpCode::Div => { let (a, b) = self.pop2()?; let v = self.div_vals(a, b)?; self.push(v); }
                OpCode::Mod => {
                let (a, b) = self.pop2()?;
                if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
                    let (_, r) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
                    let v = self.bigint_to_val(r)?;
                    self.push(v);
                } else {
                    return Err(VmErr::Type("mod requires int".into()));
                }
                }
                OpCode::Pow => {
                    let (a, b) = self.pop2()?;
                    if let Some(ba) = self.to_bigint(a) {
                        if b.is_int() {
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
                    }
                    let fa = if a.is_int() { a.as_int() as f64 } else if a.is_float() { a.as_float() }
                            else { return Err(VmErr::Type("'**' requires numeric operands".into())); };
                    let fb = if b.is_int() { b.as_int() as f64 } else if b.is_float() { b.as_float() }
                            else { return Err(VmErr::Type("'**' requires numeric operands".into())); };
                    self.push(Val::float(fpowf(fa, fb)));
                }
                OpCode::FloorDiv => {
                    let (a, b) = self.pop2()?;
                    if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
                        let (q, _) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
                        let v = self.bigint_to_val(q)?;
                        self.push(v);
                    } else {
                        return Err(VmErr::Type("// requires int".into()));
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
                            return Err(VmErr::Type("unary -".into()));
                        }
                    } else {
                        return Err(VmErr::Type("unary -".into()));
                    }
                }

                // Bitwise

                OpCode::BitAnd => { let (a,b) = self.pop2()?; self.push(Val::int(a.as_int() & b.as_int())); }
                OpCode::BitOr => { let (a,b) = self.pop2()?; self.push(Val::int(a.as_int() | b.as_int())); }
                OpCode::BitXor => { let (a,b) = self.pop2()?; self.push(Val::int(a.as_int() ^ b.as_int())); }
                OpCode::BitNot => { let v = self.pop()?; self.push(Val::int(!v.as_int())); }
                OpCode::Shl => { let (a,b) = self.pop2()?; self.push(Val::int(a.as_int() << (b.as_int() & 63))); }
                OpCode::Shr => { let (a,b) = self.pop2()?; self.push(Val::int(a.as_int() >> (b.as_int() & 63))); }

                // Comparison (cached)

                OpCode::Eq => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache, adaptive); self.push(Val::bool(self.eq_vals(a, b))); }
                OpCode::NotEq => { let (a,b) = self.pop2()?; self.push(Val::bool(!self.eq_vals(a,b))); }
                OpCode::Lt => { let (a, b) = self.pop2()?; cached_binop!(self.heap, rip, &ins.opcode, a, b, cache, adaptive); let r = self.lt_vals(a, b)?; self.push(Val::bool(r)); }
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
                    if !self.truthy(v) {
                        if self.budget == 0 { return Err(cold_budget()); }
                        self.budget -= 1;
                        let target = op as usize;
                        if target > chunk.instructions.len() { return Err(VmErr::Runtime("jump target out of bounds".into())); }
                        ip = target;
                    }
                }
                OpCode::Jump => {
                    if self.budget == 0 { return Err(cold_budget()); }
                    self.budget -= 1;
                    let target = op as usize;
                    if target > chunk.instructions.len() {
                        return Err(VmErr::Runtime("jump target out of bounds".into()));
                    }
                    ip = target;
                }
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
                OpCode::BuildDict  => {
                    let mut pairs: Vec<(Val, Val)> = Vec::with_capacity(op as usize);
                    for _ in 0..op { let v = self.pop()?; let k = self.pop()?; pairs.push((k, v)); }
                    pairs.reverse();
                    let dm = DictMap::from_pairs(pairs);
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
                OpCode::StoreItem => { self.store_item()?; }
                OpCode::UnpackSequence => {
                    let obj = self.pop()?; let expected = op as usize;
                    if !obj.is_heap() { return Err(VmErr::Type("cannot unpack non-sequence".into())); }
                    let items: Vec<Val> = match self.heap.get(obj) {
                        HeapObj::List(v) => v.borrow().clone(),
                        HeapObj::Tuple(v) => v.clone(),
                        HeapObj::Str(s) => {
                            let chars: Vec<char> = s.chars().collect();
                            if chars.len() != expected { return Err(VmErr::Value(format!("expected {} values to unpack, got {}", expected, chars.len()))); }
                            let chars = chars; drop(s);
                            let mut out = Vec::with_capacity(chars.len());
                            for c in chars { out.push(self.heap.alloc(HeapObj::Str(c.to_string()))?); }
                            out
                        }
                        _ => return Err(VmErr::Type("unpack".into())),
                    };
                    if items.len() != expected { return Err(VmErr::Value(format!("expected {} values to unpack, got {}", expected, items.len()))); }
                    for item in items.into_iter().rev() { self.push(item); }
                }
                OpCode::UnpackEx => { self.unpack_ex(op)?; }
                OpCode::FormatValue => {
                    if op == 1 { self.pop()?; }
                    let v = self.pop()?; let s = self.display(v);
                    let val = self.heap.alloc(HeapObj::Str(s))?; self.push(val);
                }

                // Iterators

                OpCode::GetIter => {
                    let obj = self.pop()?;
                    if !obj.is_heap() { return Err(VmErr::Type("not iterable".into())); }
                    let frame = match self.heap.get(obj) {
                        HeapObj::Range(s, e, st) => IterFrame::Range { cur: *s, end: *e, step: *st },
                        HeapObj::List(v)  => IterFrame::Seq { items: v.borrow().clone(), idx: 0 },
                        HeapObj::Tuple(v) => IterFrame::Seq { items: v.clone(), idx: 0 },
                        HeapObj::Dict(p) => IterFrame::Seq { items: p.borrow().keys().copied().collect(), idx: 0 },
                        HeapObj::Set(s) => IterFrame::Seq { items: s.borrow().clone(), idx: 0 },
                        HeapObj::Str(s) => {
                            let chars: Vec<char> = s.chars().collect(); drop(s);
                            let mut items = Vec::with_capacity(chars.len());
                            for c in chars { items.push(self.heap.alloc(HeapObj::Str(c.to_string()))?); }
                            IterFrame::Seq { items, idx: 0 }
                        }
                        _ => return Err(VmErr::Type("not iterable".into())),
                    };
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
                            let target = op as usize;
                            if target > chunk.instructions.len() { return Err(VmErr::Runtime("for iter target out of bounds".into())); }
                            ip = target;
                        }
                    }
                }

                // SSA Phi

                OpCode::Phi => {
                    let target = op as usize;
                    let (ia, ib) = chunk.phi_sources[phi_idx]; phi_idx += 1;
                    let val = slots[ia as usize].or(slots[ib as usize]).unwrap_or(Val::none());
                    slots[target] = Some(val);
                }

                // Functions

                OpCode::MakeFunction | OpCode::MakeCoroutine => {
                    let n_defaults = self.chunk.functions[op as usize].2 as usize;
                    let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };
                    let val = self.heap.alloc(HeapObj::Func(op as usize, defaults))?;
                    self.push(val);
                }
                OpCode::Call => {
                    let argc = op as usize;
                    if self.depth >= self.max_calls { return Err(cold_depth()); }
                    let mut args: Vec<Val> = (0..argc).map(|_| self.pop()).collect::<Result<_,_>>()?;
                    args.reverse();
                    let callee = self.pop()?;
                    if !callee.is_heap() { return Err(VmErr::Type("call non-function".into())); }
                    let (fi, captured_defaults) = match self.heap.get(callee) {
                        HeapObj::Func(i, d) => (*i, d.clone()),
                        _ => return Err(VmErr::Type("call non-function".into())),
                    };
                    if let Some(cached) = self.templates.lookup(fi, &args, &self.heap) {
                        self.push(cached); continue;
                    }
                    self.depth += 1;
                    let (params, body, _defaults, name_idx) = &self.chunk.functions[fi];
                    let mut fn_slots = self.fill_builtins(&body.names);
                    let mut body_map: HashMap<&str, usize> = HashMap::with_capacity(body.names.len());
                    for (i, n) in body.names.iter().enumerate() { body_map.insert(n.as_str(), i); }
                    // Bind args to params: detects keyword args (HeapObj::Str matching a param name) and consumes key+value pairs
                    let mut pi = 0usize;
                    for (_, p) in params.iter().enumerate() {
                        if pi >= args.len() { break; }
                        if args[pi].is_heap() {
                            if let HeapObj::Str(k) = self.heap.get(args[pi]) {
                                if params.iter().any(|p| p.trim_start_matches('*') == k.as_str()) && pi + 1 < args.len() {
                                    let pname = format!("{}_0", k);
                                    if let Some(&s) = body_map.get(pname.as_str()) { fn_slots[s] = Some(args[pi + 1]); }
                                    pi += 2;
                                    continue;
                                }
                            }
                        }
                        let pname = format!("{}_0", p.trim_start_matches('*'));
                        if let Some(&s) = body_map.get(pname.as_str()) { fn_slots[s] = Some(args[pi]); }
                        pi += 1;
                    }
                    if pi < params.len() && !captured_defaults.is_empty() {
                        let d_start = captured_defaults.len().saturating_sub(params.len() - pi);
                        for (i, param) in params[pi..].iter().enumerate() {
                            if let Some(&dv) = captured_defaults.get(d_start + i) {
                                let pname = format!("{}_0", param.trim_start_matches('*'));
                                if let Some(&s) = body_map.get(pname.as_str()) {
                                    if fn_slots[s].is_none() { fn_slots[s] = Some(dv); }
                                }
                            }
                        }
                    }
                    for (si, sv) in slots.iter().enumerate() {
                        if let Some(v) = sv {
                            if v.is_heap() {
                                if let HeapObj::Func(_, _) = self.heap.get(*v) {
                                    if let Some(&bs) = body_map.get(chunk.names[si].as_str()) { fn_slots[bs] = Some(*v); }
                                }
                            }
                        }
                    }
                    // Inject callee into body slots so the function can call itself by name
                    let name_idx = *name_idx;
                    if name_idx != u16::MAX {
                        let raw = &self.chunk.names[name_idx as usize];
                        let base = raw.rfind('_').filter(|&p| raw[p+1..].parse::<u32>().is_ok()).map(|p| &raw[..p]).unwrap_or(raw.as_str());
                        let versioned = format!("{}_0", base);
                        if let Some(&slot) = body_map.get(versioned.as_str()) {
                            if fn_slots[slot].is_none() { fn_slots[slot] = Some(callee); }
                        }
                    }

                    let yields_before = self.yields.len();
                    let snap = self.live_slots.len();
                    self.live_slots.extend(slots.iter().flatten().copied());
                    let result = self.exec(body, &mut fn_slots)?;
                    self.live_slots.truncate(snap);
                    self.depth -= 1;

                    // Collect yielded values into a list; otherwise cache pure results via templates
                    if self.yields.len() > yields_before {
                        let fn_yields = self.yields.split_off(yields_before);
                        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(fn_yields))))?;
                        self.push(val);
                    } else {
                        if body.is_pure {
                            self.templates.record(fi, &args, result, &self.heap);
                        }
                        self.push(result);
                    }
                }

                // Builtins (delegated)

                OpCode::CallPrint => { self.call_print(op)?; }
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
                OpCode::CallInput => { self.call_input()?; }
                OpCode::CallDict => { self.call_dict(op)?; }
                OpCode::CallSet => { self.call_set(op)?; }

                // Implemented stubs

                OpCode::Assert => { let v = self.pop()?; if !self.truthy(v) { return Err(VmErr::Runtime("AssertionError".into())); } }
                OpCode::Del => { let slot = op as usize; if slot < slots.len() { slots[slot] = None; } }

                // No-op stubs (safe for sandbox/WASM)

                OpCode::Global | OpCode::Nonlocal => {}
                OpCode::TypeAlias => { self.pop()?; }
                OpCode::Import => { self.push(Val::none()); }
                OpCode::ImportFrom => { self.pop()?; self.push(Val::none()); }
                OpCode::SetupExcept | OpCode::PopExcept => {}
                OpCode::Raise | OpCode::RaiseFrom => { return Err(VmErr::Runtime("exception raised".into())); }
                OpCode::SetupWith | OpCode::ExitWith => { return Err(VmErr::Runtime("with/as not yet supported".into())); }
                OpCode::Await | OpCode::YieldFrom => {}
                OpCode::UnpackArgs => { return Err(VmErr::Runtime("*args/**kwargs not yet supported".into())); }
                OpCode::MakeClass => { return Err(VmErr::Runtime("classes not yet supported".into())); }
                OpCode::LoadAttr | OpCode::StoreAttr => { return Err(VmErr::Runtime("attribute access not yet supported".into())); }
                OpCode::ListComp | OpCode::SetComp | OpCode::DictComp => { return Err(VmErr::Runtime("comprehensions not yet supported".into())); }
                OpCode::GenExpr => { return Err(VmErr::Runtime("generator expressions not yet supported".into())); }
            }
        }
    }
}