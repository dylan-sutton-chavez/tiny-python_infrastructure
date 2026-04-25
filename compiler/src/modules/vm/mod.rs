// vm/mod.rs

pub mod types;
mod cache;
mod ops;
mod builtins;
mod collections;
mod handlers;

pub use types::{Val, HeapObj, HeapPool, VmErr, Limits};

use types::*;
use cache::{OpcodeCache, FastOp, Templates};
use handlers::unsupported::unsupported;
use handlers::ControlOutcome;

use crate::modules::parser::{OpCategory, SSAChunk, Value, BUILTIN_TYPES};
use alloc::{string::{String, ToString}, vec::Vec, vec, format, boxed::Box};
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
        Routes by OpCategory to a handler. Handlers return outcomes that the loop interprets. Unsupported opcodes go through a single point.
    */

    pub(crate) fn exec(&mut self, chunk: &SSAChunk, slots: &mut [Option<Val>]) -> Result<Val, VmErr> {
        let slots_base = self.live_slots.len();
        let n = chunk.instructions.len();
        let mut cache = Box::new(OpcodeCache::new(n));
        let mut ip = 0usize;
        let prev_slots = &chunk.prev_slots;

        loop {
            if ip >= n { return Ok(Val::none()); }

            // Inline-cache fast path
            if let Some(fast) = cache.get_fast(ip) {
                ip += 1;
                if self.exec_fast(fast)? { continue; }
                cache.invalidate(ip - 1);
                ip -= 1;
            }

            if ip >= n { return Err(VmErr::Runtime("instruction pointer out of bounds")); }

            let ins = &chunk.instructions[ip];
            let op = ins.operand;
            let rip = ip;
            ip += 1;

            // Group categorization for OpCode.
            match ins.opcode.category() {
                OpCategory::Load => self.handle_load(ins.opcode, op, chunk, slots)?,
                OpCategory::Store => {
                    self.handle_store(op, slots, prev_slots)?;
                    if self.heap.needs_gc() { self.collect(slots); }
                }
                OpCategory::Arith => self.handle_arith(ins.opcode, rip, &mut cache)?,
                OpCategory::Bitwise => self.handle_bitwise(ins.opcode)?,
                OpCategory::Compare => self.handle_compare(ins.opcode, rip, &mut cache)?,
                OpCategory::Logic => self.handle_logic(ins.opcode)?,
                OpCategory::Identity => self.handle_identity(ins.opcode)?,
                OpCategory::ControlFlow => match self.handle_control(ins.opcode, op, n)? {
                    ControlOutcome::Continue => {}
                    ControlOutcome::Jump(t) => ip = t,
                    ControlOutcome::Return(v) => {
                        self.live_slots.truncate(slots_base);
                        return Ok(v);
                    }
                },
                OpCategory::Iter => match self.handle_iter(ins.opcode, op, n, slots)? {
                    ControlOutcome::Continue => {}
                    ControlOutcome::Jump(t) => ip = t,
                    ControlOutcome::Return(_) => unreachable!("iter never returns"),
                },
                OpCategory::Build => self.handle_build(ins.opcode, op)?,
                OpCategory::Container => self.handle_container(ins.opcode, op)?,
                OpCategory::Comprehension => self.handle_comprehension(ins.opcode)?,
                OpCategory::Function => self.handle_function(ins.opcode, op, chunk, slots)?,
                OpCategory::Ssa => Self::exec_phi(op, rip, &chunk.phi_map, slots, prev_slots, &chunk.phi_sources),
                OpCategory::Yield => self.handle_yield()?,
                OpCategory::Side => self.handle_side(ins.opcode, op, slots)?,
                OpCategory::Unsupported => return Err(unsupported(ins.opcode)),
            }
        }
    }
}