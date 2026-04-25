// vm/mod.rs

pub mod types;
mod cache;
mod ops;
mod builtins;
mod collections;
mod handlers;
mod super_ops;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, FastOp, Templates};
use handlers::unsupported::unsupported;
use handlers::ControlOutcome;

use crate::modules::parser::{SSAChunk, Value, BUILTIN_TYPES};
use alloc::{string::{String, ToString}, vec::Vec, vec, format};
use crate::modules::fx::FxHashMap as HashMap;

/*
VM State
    Stack, heap, iterators, yield buffer, templates and sandbox counters.
*/

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
            HeapObj::List(v) => v.borrow().clone(),
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
        Three-tier dispatch: superinstruction (tier-2) -> inline cache (tier-1) -> flat opcode match (tier-0). LLVM lowers the flat match to a single jump table; cache lives on the exec stack frame to avoid heap traffic.
    */

    pub(crate) fn exec(&mut self, chunk: &SSAChunk, slots: &mut [Option<Val>]) -> Result<Val, VmErr> {
        use super_ops::{FusedOutcome, SuperOp};

        let slots_base = self.live_slots.len();
        let n = chunk.instructions.len();
        let mut cache = OpcodeCache::new(chunk);  // stack-allocated, no Box
        let mut ip = 0usize;
        let prev_slots = chunk.prev_slots.as_slice();
        let instructions = chunk.instructions.as_slice();

        loop {
            if ip >= n { return Ok(Val::none()); }

            // ── Tier-2: superinstruction dispatch ─────────────────────────
            if let Some(sop) = cache.get_super(ip) {
                match sop {
                    SuperOp::Inc { load, store, delta, len } => {
                        if super_ops::super_inc(slots, prev_slots, load, store, delta) {
                            ip += len as usize; continue;
                        }
                    }
                    SuperOp::Lt { a, b, len } => {
                        let r = super_ops::super_lt(slots, a, b);
                        if r != -1 {
                            self.push(Val::bool(r == 1));
                            ip += len as usize; continue;
                        }
                    }
                    SuperOp::LoopGuard { load, store, delta, limit, jump_target, len } => {
                        let r = super_ops::super_loop_guard(slots, prev_slots, load, store, delta, limit);
                        if r != -1 {
                            if r == 1 { ip += len as usize; }
                            else {
                                if self.budget == 0 { return Err(cold_budget()); }
                                self.budget -= 1;
                                if jump_target as usize > n {
                                    return Err(VmErr::Runtime("jump target out of bounds"));
                                }
                                ip = jump_target as usize;
                            }
                            continue;
                        }
                    }
                    SuperOp::RangeIncFused { drop_slot, counter_load, counter_store, delta, end_ip } => {
                        if let Some(iter) = self.iter_stack.last() {
                            let outcome = super_ops::run_range_inc_fused(
                                slots, prev_slots, iter, &mut self.budget,
                                drop_slot, counter_load, counter_store, delta,
                            );
                            if let FusedOutcome::Done = outcome {
                                self.iter_stack.pop();
                                if end_ip as usize > n {
                                    return Err(VmErr::Runtime("jump target out of bounds"));
                                }
                                ip = end_ip as usize;
                                continue;
                            }
                        }
                    }
                }
            }

            // ── Tier-1: IC fast path ──────────────────────────────────────
            if let Some(fast) = cache.get_fast(ip) {
                ip += 1;
                if self.exec_fast(fast)? { continue; }
                cache.invalidate(ip - 1);
                ip -= 1;
            }

            // ── Tier-0: direct opcode dispatch (FLAT MATCH) ────────────────
            // SAFETY: ip < n checked at loop top; instructions is chunk slice.
            let ins = unsafe { instructions.get_unchecked(ip) };
            let op = ins.operand;
            let rip = ip;
            ip += 1;

            // ONE indirect jump. LLVM emits jump table directly.
            // Hot opcodes first to bias branch prediction in the dense fall-through.
            match ins.opcode {
                // ── HOT: numeric ops (most frequent in loops) ──────────────
                OpCode::LoadName => {
                    let slot = op as usize;
                    self.push(slots[slot].ok_or_else(|| VmErr::Name(chunk.names[slot].clone()))?);
                }
                OpCode::StoreName => {
                    self.handle_store(op, slots, prev_slots)?;
                    if self.heap.needs_gc() { self.collect(slots); }
                }
                OpCode::LoadConst => {
                    let v = self.val_from(&chunk.constants[op as usize])?;
                    self.push(v);
                }
                OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
                | OpCode::Mod | OpCode::Pow | OpCode::FloorDiv | OpCode::Minus => {
                    self.handle_arith(ins.opcode, rip, &mut cache)?;
                }
                OpCode::Eq | OpCode::NotEq | OpCode::Lt | OpCode::Gt
                | OpCode::LtEq | OpCode::GtEq => {
                    self.handle_compare(ins.opcode, rip, &mut cache)?;
                }
                OpCode::Jump => {
                    ip = self.checked_jump(op as usize, n)?;
                }
                OpCode::JumpIfFalse => {
                    let v = self.pop()?;
                    if !self.truthy(v) { ip = self.checked_jump(op as usize, n)?; }
                }
                OpCode::ForIter => {
                    if self.budget == 0 { return Err(cold_budget()); }
                    self.budget -= 1;
                    if self.heap.needs_gc() { self.collect(slots); }
                    match self.iter_stack.last_mut().and_then(|f| f.next_item()) {
                        Some(item) => self.push(item),
                        None => {
                            self.iter_stack.pop();
                            if op as usize > n {
                                return Err(VmErr::Runtime("jump target out of bounds"));
                            }
                            ip = op as usize;
                        }
                    }
                }
                OpCode::PopTop => { self.pop()?; }
                OpCode::ReturnValue => {
                    let result = if self.stack.is_empty() { Val::none() } else { self.pop()? };
                    self.live_slots.truncate(slots_base);
                    return Ok(result);
                }

                // ── WARM: container ops, builtins ─────────────────────────
                OpCode::GetItem => { self.get_item()?; }
                OpCode::Call => self.handle_function(ins.opcode, op, chunk, slots)?,
                OpCode::CallPrint | OpCode::CallLen | OpCode::CallAbs | OpCode::CallStr
                | OpCode::CallInt | OpCode::CallFloat | OpCode::CallBool | OpCode::CallType
                | OpCode::CallChr | OpCode::CallOrd | OpCode::CallSorted | OpCode::CallList
                | OpCode::CallTuple | OpCode::CallEnumerate | OpCode::CallIsInstance
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
                OpCode::And | OpCode::Or | OpCode::Not => self.handle_logic(ins.opcode)?,
                OpCode::In | OpCode::NotIn | OpCode::Is | OpCode::IsNot => self.handle_identity(ins.opcode)?,
                OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor | OpCode::BitNot
                | OpCode::Shl | OpCode::Shr => self.handle_bitwise(ins.opcode)?,

                // ── COLD: everything else falls through to handlers ────────
                OpCode::BuildList | OpCode::BuildTuple | OpCode::BuildDict
                | OpCode::BuildString | OpCode::BuildSet | OpCode::BuildSlice => {
                    self.handle_build(ins.opcode, op)?;
                }
                OpCode::StoreItem | OpCode::UnpackSequence | OpCode::UnpackEx
                | OpCode::FormatValue => {
                    self.handle_container(ins.opcode, op)?;
                }
                OpCode::ListAppend | OpCode::SetAdd | OpCode::MapAdd => {
                    self.handle_comprehension(ins.opcode)?;
                }
                OpCode::Phi => Self::exec_phi(op, rip, &chunk.phi_map, slots, prev_slots, &chunk.phi_sources),
                OpCode::Yield => self.handle_yield()?,
                OpCode::LoadEllipsis => {
                    let v = self.heap.alloc(HeapObj::Str("...".to_string()))?;
                    self.push(v);
                }
                OpCode::Dup2 => {
                    let b = self.pop()?; let a = self.pop()?;
                    self.push(a); self.push(b); self.push(a); self.push(b);
                }
                OpCode::Assert | OpCode::Del | OpCode::Global | OpCode::Nonlocal
                | OpCode::TypeAlias | OpCode::Import | OpCode::ImportFrom
                | OpCode::SetupExcept | OpCode::PopExcept | OpCode::Raise | OpCode::RaiseFrom
                | OpCode::Await | OpCode::YieldFrom => {
                    self.handle_side(ins.opcode, op, slots)?;
                }
                OpCode::MakeClass | OpCode::LoadAttr | OpCode::StoreAttr
                | OpCode::SetupWith | OpCode::ExitWith | OpCode::UnpackArgs => {
                    return Err(unsupported(ins.opcode));
                }
            }
        }
    }
}