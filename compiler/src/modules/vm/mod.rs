// vm/mod.rs

pub mod types;
mod cache;
mod ops;
mod builtins;
mod handlers;
pub mod optimizer;

use crate::s;
use crate::modules::parser::{OpCode, SSAChunk, Value, BUILTIN_TYPES};
use crate::modules::fx::FxHashMap as HashMap;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, FastOp, Templates};
use alloc::{string::{String, ToString}, vec::Vec, vec};

/* Stack, heap, iterators, yield buffer, templates and sandbox counters. */

pub(crate) struct ExceptionFrame {
    pub handler_ip: usize,
    pub stack_depth: usize,
    pub iter_depth: usize,
    pub with_depth: usize,
}

pub struct VM<'a> {
    pub(crate) stack: Vec<Val>,
    pub(crate) heap: HeapPool,
    pub(crate) iter_stack: Vec<IterFrame>,
    pub(crate) yields: Vec<Val>,
    pub(crate) chunk: &'a SSAChunk,
    pub(crate) globals: HashMap<String, Val>,
    pub(crate) live_slots: Vec<Val>,
    pub(crate) templates: Templates,
    pub(crate) budget: usize,
    pub(crate) depth: usize,
    pub(crate) max_calls: usize,
    pub(crate) observed_impure: Vec<bool>,
    pub(crate) exception_stack: Vec<ExceptionFrame>,
    pub(crate) functions: Vec<&'a (Vec<String>, SSAChunk, u16, u16)>,
    pub(crate) fn_index: HashMap<*const SSAChunk, Vec<u32>>,
    pub(crate) opcode_caches: HashMap<*const SSAChunk, OpcodeCache>,
    pub(crate) with_stack: Vec<Val>,
    pub(crate) pending_pos_delta: i32,
    pub(crate) pending_kw_delta: i32,
    pub output: Vec<String>,
}

impl<'a> VM<'a> {
    pub fn new(chunk: &'a SSAChunk) -> Self { Self::with_limits(chunk, Limits::none()) }

    /// One-shot recursive flatten of nested `def`s. Each chunk's local function
    /// table maps to a contiguous range of global ids; nested bodies are walked
    /// depth-first so a closure defined inside a nested function still resolves.
    fn build_function_table(&mut self, chunk: &'a SSAChunk) {
        let mut indices = Vec::with_capacity(chunk.functions.len());
        for desc in chunk.functions.iter() {
            let global = self.functions.len() as u32;
            self.functions.push(desc);
            indices.push(global);
            self.build_function_table(&desc.1);
        }
        self.fn_index.insert(chunk as *const _, indices);
    }

    /// Materializa un iterable en `Vec<Val>` para spread posicional.
    fn iter_to_vec_for_spread(&self, v: Val) -> Result<Vec<Val>, VmErr> {
        if !v.is_heap() {
            return Err(VmErr::Type("argument after * must be an iterable"));
        }
        Ok(match self.heap.get(v) {
            HeapObj::List(rc)  => rc.borrow().clone(),
            HeapObj::Tuple(t)  => t.clone(),
            HeapObj::Set(rc)   => rc.borrow().iter().cloned().collect(),
            HeapObj::Range(s, e, st) => {
                let (s, e, st) = (*s, *e, *st);
                if st == 0 { return Err(VmErr::Value("range() arg 3 must not be zero")); }
                let mut out = Vec::new();
                let mut i = s;
                if st > 0 { while i < e { out.push(Val::int(i)); i += st; } }
                else      { while i > e { out.push(Val::int(i)); i += st; } }
                out
            }
            _ => return Err(VmErr::Type("argument after * must be an iterable")),
        })
    }

    /// Materializa un mapping en pares (clave_str, valor) para spread por nombre.
    fn mapping_to_kw_pairs(&self, v: Val) -> Result<Vec<(Val, Val)>, VmErr> {
        if !v.is_heap() {
            return Err(VmErr::Type("argument after ** must be a mapping"));
        }
        match self.heap.get(v) {
            HeapObj::Dict(rc) => {
                let entries: Vec<(Val, Val)> = rc.borrow().iter().collect();
                for (k, _) in &entries {
                    if !k.is_heap() || !matches!(self.heap.get(*k), HeapObj::Str(_)) {
                        return Err(VmErr::Type("keywords must be strings"));
                    }
                }
                Ok(entries)
            }
            _ => Err(VmErr::Type("argument after ** must be a mapping")),
        }
    }

    fn fill_builtins(&self, names: &[String]) -> Vec<Option<Val>> {
        let mut slots = vec![None; names.len()];
        for (i, name) in names.iter().enumerate() {
            if let Some(v) = self.globals.get(name) {
                slots[i] = Some(*v);
            }
        }
        slots
    }

    #[inline]
    fn checked_jump(&mut self, target: usize, limit: usize) -> Result<usize, VmErr> {
        if self.budget == 0 { return Err(cold_budget()); }
        self.budget -= 1;
        if target > limit { return Err(cold_runtime("jump target out of bounds")); }
        Ok(target)
    }

    pub(crate) fn str_to_char_vals(&mut self, s: &str) -> Result<Vec<Val>, VmErr> {
        s.chars().map(|c| self.heap.alloc(HeapObj::Str(c.to_string()))).collect()
    }

    fn make_iter_frame(&mut self, obj: Val) -> Result<IterFrame, VmErr> {
        if !obj.is_heap() { return Err(cold_type("object is not iterable")); }
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
                        (true, false) => core::cmp::Ordering::Less,
                        (false, true) => core::cmp::Ordering::Greater,
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
            _ => return Err(cold_type("object is not iterable")),
        })
    }

    fn exec_unpack_seq(&mut self, expected: usize) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(cold_type("cannot unpack non-sequence")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v) => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            HeapObj::Str(s) => {
                let s = s.clone();
                let out = self.str_to_char_vals(&s)?;
                if out.len() != expected {
                    return Err(cold_value("not enough values to unpack"));
                }
                out
            },
            _ => return Err(cold_type("cannot unpack non-sequence")),
        };
        if items.len() != expected {
            return Err(cold_value("not enough values to unpack"));
        }
        for item in items.into_iter().rev() { self.push(item); }
        Ok(())
    }

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
            with_stack: Vec::new(),
            pending_pos_delta: 0,
            pending_kw_delta: 0,
            output: Vec::new(),
            observed_impure: Vec::new(),
            exception_stack: Vec::new(),
            functions: Vec::new(),
            fn_index: HashMap::default(),
            opcode_caches: HashMap::default(),
        };
        vm.build_function_table(chunk);
        for &name in BUILTIN_TYPES {
            if let Ok(type_obj) = vm.heap.alloc(HeapObj::Type(name.to_string())) {
                vm.globals.insert(name.to_string(), type_obj);
                vm.globals.insert(s!(str name, "_0"), type_obj);
            }
        }
        vm
    }

    pub fn run(&mut self) -> Result<Val, VmErr> {
        let mut slots = self.fill_builtins(&self.chunk.names);
        self.exec(self.chunk, &mut slots)
    }

    fn collect(&mut self, current_slots: &[Option<Val>]) {
        for &v in &self.stack { self.heap.mark(v); }
        for &v in &self.with_stack { self.heap.mark(v); }
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

    /* Stack helpers */

    #[inline] pub(crate) fn push(&mut self, v: Val) { self.stack.push(v); }

    #[inline] pub(crate) fn pop(&mut self) -> Result<Val, VmErr> {
        self.stack.pop().ok_or(cold_runtime("stack underflow"))
    }
    #[inline] pub(crate) fn pop2(&mut self) -> Result<(Val, Val), VmErr> {
        let b = self.pop()?; let a = self.pop()?; Ok((a, b))
    }
    #[inline] pub(crate) fn pop_n(&mut self, n: usize) -> Result<Vec<Val>, VmErr> {
        let at = self.stack.len().checked_sub(n)
            .ok_or(cold_runtime("stack underflow"))?;
        Ok(self.stack.split_off(at))
    }

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

    /// Fast-path executor; peeks stack without popping. Returns false (with
    /// stack untouched) on type-guard miss so caller can deopt.
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
            FastOp::GtInt if a.is_int() && b.is_int() => Val::bool(a.as_int() > b.as_int()),
            FastOp::LtEqInt if a.is_int() && b.is_int() => Val::bool(a.as_int() <= b.as_int()),
            FastOp::GtEqInt if a.is_int() && b.is_int() => Val::bool(a.as_int() >= b.as_int()),
            FastOp::NotEqInt if a.is_int() && b.is_int() => Val::bool(a.as_int() != b.as_int()),

            FastOp::AddStr | FastOp::EqStr if a.is_heap() && b.is_heap() => {
                let (sa, sb) = match (self.heap.get(a), self.heap.get(b)) {
                    (HeapObj::Str(x), HeapObj::Str(y)) => (x.clone(), y.clone()),
                    _ => return Ok(false),
                };
                match fast {
                    FastOp::AddStr => {
                        let mut r = String::with_capacity(sa.len() + sb.len());
                        r.push_str(&sa); r.push_str(&sb);
                        self.heap.alloc(HeapObj::Str(r))?
                    }
                    _ => Val::bool(sa == sb),
                }
            }

            _ => return Ok(false),
        };

        self.stack.truncate(len - 2);
        self.push(result);
        Ok(true)
    }

    /// Main dispatch loop. Walks the fused instruction stream (LoadAttr+Call
    /// already collapsed to CallMethod+CallMethodArgs). IC is checked inline
    /// for hot arith/compare opcodes.
    pub(crate) fn exec(&mut self, chunk: &SSAChunk, slots: &mut [Option<Val>]) -> Result<Val, VmErr> {
        let slots_base = self.live_slots.len();
        let exc_base   = self.exception_stack.len();
        let key        = chunk as *const _;

        let mut cache = self.opcode_caches.remove(&key)
            .unwrap_or_else(|| OpcodeCache::new(chunk));
        cache.ensure_fused(chunk);

        let result: Result<Val, VmErr> = (|| {
            let n          = cache.fused_ref().len();
            let mut ip     = 0usize;
            let prev_slots = chunk.prev_slots.as_slice();

            loop {
                if ip >= n {
                    self.exception_stack.truncate(exc_base);
                    return Ok(Val::none());
                }

                match self.dispatch(chunk, slots, &mut cache, &mut ip, n, prev_slots) {
                    Ok(None) => {}
                    Ok(Some(v)) => {
                        self.live_slots.truncate(slots_base);
                        self.exception_stack.truncate(exc_base);
                        return Ok(v);
                    }
                    Err(e) => {
                        if self.exception_stack.len() > exc_base {
                            let frame = self.exception_stack.pop().unwrap();
                            self.stack.truncate(frame.stack_depth);
                            self.iter_stack.truncate(frame.iter_depth);
                            self.with_stack.truncate(frame.with_depth);
                            self.pending_pos_delta = 0;
                            self.pending_kw_delta  = 0;
                            let msg = match &e {
                                VmErr::ZeroDiv    => "ZeroDivisionError",
                                VmErr::Type(_)    => "TypeError",
                                VmErr::Value(_)   => "ValueError",
                                VmErr::Name(_)    => "NameError",
                                VmErr::CallDepth  => "RecursionError",
                                VmErr::Heap       => "MemoryError",
                                VmErr::Budget     => "RuntimeError",
                                VmErr::Runtime(_) => "RuntimeError",
                                VmErr::Raised(_)  => "Exception",
                            };
                            let exc = self.heap.alloc(HeapObj::Str(msg.to_string()))?;
                            self.push(exc);
                            ip = frame.handler_ip;
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        })();

        self.opcode_caches.insert(key, cache);
        result
    }

    /// CallMethod: resolves the bound method on the receiver and calls it
    /// directly, no BoundMethod heap alloc. Reads args from CallMethodArgs.
    fn exec_call_method(&mut self, attr_idx: u16, call_op: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let raw = call_op as usize;
        let num_kw  = (raw >> 8) & 0xFF;
        let num_pos = raw & 0xFF;
        let total = num_pos + 2 * num_kw;

        let mut stack_items: Vec<Val> = (0..total)
            .map(|_| self.pop())
            .collect::<Result<_, _>>()?;
        stack_items.reverse();

        let kw_flat: Vec<Val> = stack_items.split_off(num_pos);
        let positional = stack_items;

        let obj = self.pop()?;
        let ty = self.type_name(obj);
        let name = chunk.names.get(attr_idx as usize)
            .ok_or(VmErr::Runtime("CallMethod: bad name index"))?;

        let method_id = handlers::methods::lookup_method(ty, name.as_str())
            .ok_or(VmErr::Type("'object' has no attribute"))?;

        self.exec_bound_method(obj, method_id, positional, kw_flat)
    }

    #[inline]
    fn dispatch(
        &mut self, chunk: &SSAChunk, slots: &mut [Option<Val>],
        cache: &mut OpcodeCache, ip: &mut usize, n: usize,
        prev_slots: &[Option<u16>],
    ) -> Result<Option<Val>, VmErr> {
        let ins = cache.fused_ref()[*ip];
        let rip = *ip;
        let op = ins.operand;
        *ip += 1;

        match ins.opcode {
            // ── SHORT-CIRCUIT JUMPS ───────────────────────────────────
            OpCode::JumpIfFalseOrPop => {
                let v = *self.stack.last().ok_or(cold_runtime("stack underflow"))?;
                if !self.truthy(v) { *ip = op as usize; }
                else { self.pop()?; }
            }
            OpCode::JumpIfTrueOrPop => {
                let v = *self.stack.last().ok_or(cold_runtime("stack underflow"))?;
                if self.truthy(v) { *ip = op as usize; }
                else { self.pop()?; }
            }

            // ── HOT OPCODES ───────────────────────────────────────────
            OpCode::LoadName => {
                self.push(slots[op as usize].ok_or_else(|| VmErr::Name(chunk.names[op as usize].clone()))?);
            }
            OpCode::StoreName => {
                self.handle_store(op, slots, prev_slots)?;
                if self.heap.needs_gc() { self.collect(slots); }
            }
            OpCode::LoadConst => {
                let v = chunk.constants.get(op as usize)
                    .ok_or(cold_runtime("constant index out of bounds"))
                    .and_then(|c| self.val_from(c))?;
                self.push(v);
            }

            // Arith with IC
            OpCode::Add | OpCode::Sub | OpCode::Mul => {
                if let Some(fast) = cache.get_fast(rip) {
                    if self.exec_fast(fast)? { return Ok(None); }
                    cache.invalidate(rip);
                }
                self.handle_arith(ins.opcode, rip, cache)?;
            }
            OpCode::Div | OpCode::Mod | OpCode::Pow | OpCode::FloorDiv | OpCode::Minus => {
                self.handle_arith(ins.opcode, rip, cache)?;
            }

            // Compare with IC
            OpCode::Eq | OpCode::Lt => {
                if let Some(fast) = cache.get_fast(rip) {
                    if self.exec_fast(fast)? { return Ok(None); }
                    cache.invalidate(rip);
                }
                self.handle_compare(ins.opcode, rip, cache)?;
            }
            OpCode::NotEq | OpCode::Gt | OpCode::LtEq | OpCode::GtEq => {
                self.handle_compare(ins.opcode, rip, cache)?;
            }

            OpCode::Jump => { *ip = self.checked_jump(op as usize, n)?; }
            OpCode::JumpIfFalse => {
                let v = self.pop()?;
                if !self.truthy(v) { *ip = self.checked_jump(op as usize, n)?; }
            }
            OpCode::ForIter => {
                if self.budget == 0 { return Err(cold_budget()); }
                self.budget -= 1;
                if self.heap.needs_gc() { self.collect(slots); }
                match self.iter_stack.last_mut().and_then(|f| f.next_item()) {
                    Some(item) => self.push(item),
                    None => {
                        self.iter_stack.pop();
                        if op as usize > n { return Err(cold_runtime("jump target out of bounds")); }
                        *ip = op as usize;
                    }
                }
            }
            OpCode::PopTop => { self.pop()?; }
            OpCode::ReturnValue => {
                let result = if self.stack.is_empty() { Val::none() } else { self.pop()? };
                return Ok(Some(result));
            }

            // ── WARM OPCODES ──────────────────────────────────────────
            OpCode::GetItem => { self.get_item()?; }

            OpCode::Call | OpCode::CallPrint | OpCode::CallLen | OpCode::CallAbs
            | OpCode::CallStr | OpCode::CallInt | OpCode::CallFloat | OpCode::CallBool
            | OpCode::CallType | OpCode::CallChr | OpCode::CallOrd | OpCode::CallSorted
            | OpCode::CallList | OpCode::CallTuple | OpCode::CallEnumerate | OpCode::CallIsInstance
            | OpCode::CallRange | OpCode::CallRound | OpCode::CallMin | OpCode::CallMax
            | OpCode::CallSum | OpCode::CallZip | OpCode::CallDict | OpCode::CallSet
            | OpCode::CallInput | OpCode::MakeFunction | OpCode::MakeCoroutine => {
                self.handle_function(ins.opcode, op, chunk, slots)?;
            }

            OpCode::GetIter => {
                let obj = self.pop()?;
                let frame = self.make_iter_frame(obj)?;
                self.iter_stack.push(frame);
            }
            OpCode::LoadTrue  => self.push(Val::bool(true)),
            OpCode::LoadFalse => self.push(Val::bool(false)),
            OpCode::LoadNone  => self.push(Val::none()),
            OpCode::Not => self.handle_logic(OpCode::Not)?,

            OpCode::Phi => {
                Self::exec_phi(op, rip, &chunk.phi_map, slots, prev_slots, &chunk.phi_sources);
            }

            OpCode::LoadAttr => self.handle_load_attr(op, chunk)?,

            // ── FUSED METHOD CALL ─────────────────────────────────────
            OpCode::CallMethod => {
                // The next instruction is CallMethodArgs carrying the call op.
                let call_op = cache.fused_ref()[*ip].operand;
                *ip += 1; // consume CallMethodArgs
                self.exec_call_method(op, call_op, chunk)?;
            }
            OpCode::CallMethodArgs => {
                // Should never be reached on its own — always consumed by CallMethod.
                return Err(cold_runtime("CallMethodArgs reached dispatch unpaired"));
            }

            // ── COLD OPCODES ──────────────────────────────────────────
            OpCode::And | OpCode::Or => {
                return Err(cold_runtime("And/Or reached VM dispatch (should be short-circuited)"));
            }

            other => self.dispatch_generic(other, op, slots)?,
        }
        Ok(None)
    }

    fn dispatch_generic(
        &mut self, opcode: OpCode, operand: u16,
        slots: &mut [Option<Val>],
    ) -> Result<(), VmErr> {
        match opcode {
            OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor
            | OpCode::BitNot | OpCode::Shl | OpCode::Shr => self.handle_bitwise(opcode)?,
            OpCode::In | OpCode::NotIn | OpCode::Is | OpCode::IsNot => self.handle_identity(opcode)?,

            OpCode::BuildList | OpCode::BuildTuple | OpCode::BuildDict
            | OpCode::BuildString | OpCode::BuildSet | OpCode::BuildSlice => self.handle_build(opcode, operand)?,

            OpCode::StoreItem => { self.mark_impure(); self.store_item()?; }
            OpCode::UnpackSequence | OpCode::UnpackEx | OpCode::FormatValue => self.handle_container(opcode, operand)?,

            OpCode::ListAppend | OpCode::SetAdd | OpCode::MapAdd => self.handle_comprehension(opcode)?,

            OpCode::Yield => self.handle_yield()?,
            OpCode::LoadEllipsis => {
                let v = self.heap.alloc(HeapObj::Str("...".to_string()))?;
                self.push(v);
            }
            OpCode::Dup => {
                let v = *self.stack.last().ok_or(cold_runtime("stack underflow"))?;
                self.push(v);
            }
            OpCode::Dup2 => {
                let b = self.pop()?; let a = self.pop()?;
                self.push(a); self.push(b); self.push(a); self.push(b);
            }
            OpCode::Assert | OpCode::Del | OpCode::Global | OpCode::Nonlocal
            | OpCode::TypeAlias | OpCode::Import | OpCode::ImportFrom
            | OpCode::Raise | OpCode::RaiseFrom | OpCode::Await | OpCode::YieldFrom => {
                self.handle_side(opcode, operand, slots)?;
            }
            OpCode::SetupExcept => {
                self.exception_stack.push(ExceptionFrame {
                    handler_ip:  operand as usize,
                    stack_depth: self.stack.len(),
                    iter_depth:  self.iter_stack.len(),
                    with_depth:  self.with_stack.len(),
                });
            }
            OpCode::SetupWith => {
                let _ = operand;
                let cm = self.pop()?;
                self.with_stack.push(cm);
                self.push(cm);
            }
            OpCode::ExitWith => {
                let _ = operand;
                let cm = self.with_stack.pop()
                    .ok_or(cold_runtime("ExitWith without matching SetupWith"))?;
                if let Some(&top) = self.stack.last()
                    && top.0 == cm.0 { self.pop()?; }
            }
            OpCode::UnpackArgs => {
                let val = self.pop()?;
                match operand {
                    1 => {
                        let items = self.iter_to_vec_for_spread(val)?;
                        let n = items.len() as i32;
                        for v in items { self.push(v); }
                        self.pending_pos_delta += n - 1;
                    }
                    2 => {
                        let pairs = self.mapping_to_kw_pairs(val)?;
                        let n = pairs.len() as i32;
                        for (k, v) in pairs { self.push(k); self.push(v); }
                        self.pending_pos_delta -= 1;
                        self.pending_kw_delta  += n;
                    }
                    _ => return Err(cold_runtime("UnpackArgs: bad operand")),
                }
            }
            OpCode::PopExcept => { self.exception_stack.pop(); }
            OpCode::MakeClass | OpCode::StoreAttr => {
                return Err(VmErr::Runtime("objects not yet supported"));
            }
            _ => return Err(cold_runtime("unexpected opcode in generic dispatch")),
        }
        Ok(())
    }
}