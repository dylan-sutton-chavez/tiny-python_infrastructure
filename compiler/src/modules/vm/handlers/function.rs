// vm/handlers/function.rs

use super::*;
use crate::alloc::string::ToString;

/// String method with no arguments: recv_str → transform → alloc Str → push
macro_rules! str_no_args {
    ($self:ident, $recv:ident, $positional:ident, $method:literal, $transform:expr) => {{
        expect_args!($positional, 0, $method);
        let s   = $self.recv_str($recv)?;
        let out = $transform(s);
        let val = $self.heap.alloc(HeapObj::Str(out))?;
        $self.push(val);
        Ok(())
    }};
}

/// Validates exactly N arguments, unifying error messages across the VM.
macro_rules! expect_args {
    ($positional:ident, $n:expr, $name:literal) => {
        if $positional.len() != $n {
            return Err(VmErr::Type(concat!(
                $name, "() takes exactly ", stringify!($n), " argument(s)"
            )));
        }
    };
}

/// String method with 1 string argument → produces a value (like bool or int)
macro_rules! str_one_str_arg {
    ($self:ident, $recv:ident, $positional:ident, $name:literal, $body:expr) => {{
        expect_args!($positional, 1, $name);
        let s   = $self.recv_str($recv)?;
        let arg = $self.val_to_str($positional[0])?;
        let result = $body(s, arg);
        $self.push(result);
        Ok(())
    }};
}

impl<'a> VM<'a> {
    pub(crate) fn handle_function(
        &mut self, op: OpCode, operand: u16,
        chunk: &SSAChunk, slots: &mut [Option<Val>]
    ) -> Result<(), VmErr> {
        match op {
            // User functions
            OpCode::Call => self.exec_call(operand, chunk, slots),
            OpCode::MakeFunction | OpCode::MakeCoroutine => self.exec_make_function(operand, chunk, slots),

            // Pure built-ins
            OpCode::CallLen => self.call_len(),
            OpCode::CallAbs => self.call_abs(),
            OpCode::CallStr => self.call_str(),
            OpCode::CallInt => self.call_int(),
            OpCode::CallFloat => self.call_float(),
            OpCode::CallBool => self.call_bool(),
            OpCode::CallType => self.call_type(),
            OpCode::CallChr => self.call_chr(),
            OpCode::CallOrd => self.call_ord(),
            OpCode::CallSorted => self.call_sorted(),
            OpCode::CallList => self.call_list(),
            OpCode::CallTuple => self.call_tuple(),
            OpCode::CallEnumerate => self.call_enumerate(),
            OpCode::CallIsInstance => self.call_isinstance(),
            OpCode::CallRange => self.call_range(operand),
            OpCode::CallRound => self.call_round(operand),
            OpCode::CallMin => self.call_min(operand),
            OpCode::CallMax => self.call_max(operand),
            OpCode::CallSum => self.call_sum(operand),
            OpCode::CallZip => self.call_zip(operand),
            OpCode::CallDict => self.call_dict(operand),
            OpCode::CallSet => self.call_set(operand),

            // ── Impure built-ins ──────────────────────────────────────────
            OpCode::CallPrint => { self.mark_impure(); self.call_print(operand) }
            OpCode::CallInput => { self.mark_impure(); self.call_input() }

            // Not recognized here, but also not a function opcode. Defer to unsupported handler.
            _ => unreachable!("non-function opcode in handle_function"),
        }
    }

    fn exec_make_function(&mut self, operand: u16, chunk: &SSAChunk, slots: &[Option<Val>]) -> Result<(), VmErr> {
        let global = self.fn_index
            .get(&(chunk as *const _))
            .and_then(|v| v.get(operand as usize).copied())
            .ok_or(VmErr::Runtime("MakeFunction: unknown function index"))? as usize;

        let n_defaults = self.functions[global].2 as usize;
        let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };

        // Build chunk name -> slot index map for capture lookup.
        let chunk_map: HashMap<&str, usize> = chunk.names.iter()
            .enumerate().map(|(i, n)| (n.as_str(), i)).collect();

        // Capture free variables from the enclosing frame at function-creation time.
        let (params, body, _, _) = self.functions[global];
        let param_names: alloc::collections::BTreeSet<String> = params.iter()
            .map(|p| format!("{}_0", p.trim_start_matches('*')))
            .collect();
        let mut captures: Vec<(usize, Val)> = Vec::new();
        for (bi, bname) in body.names.iter().enumerate() {
            if param_names.contains(bname.as_str()) { continue; }
            if let Some(&si) = chunk_map.get(bname.as_str())
                && let Some(Some(v)) = slots.get(si) {
                    captures.push((bi, *v));
                }
        }

        let val = self.heap.alloc(HeapObj::Func(global, defaults, captures))?;
        self.push(val);
        Ok(())
    }

    fn exec_call(&mut self, operand: u16, chunk: &SSAChunk, slots: &mut [Option<Val>]) -> Result<(), VmErr> {
        let raw = operand as usize;
        let num_kw  = (raw >> 8) & 0xFF;
        let num_pos = raw & 0xFF;
        let total_items = num_pos + 2 * num_kw;

        if self.depth >= self.max_calls { return Err(cold_depth()); }

        let mut stack_items: Vec<Val> = (0..total_items)
            .map(|_| self.pop())
            .collect::<Result<_, _>>()?;
        stack_items.reverse();

        let kw_flat: Vec<Val> = stack_items.split_off(num_pos);
        let positional = stack_items;

        let callee = self.pop()?;
        if !callee.is_heap() { return Err(VmErr::Type("object is not callable")); }

        // Bound methods short-circuit the user-function machinery.
        // No SSA frame, no template cache, no recursion budget — they
        // mutate the receiver directly and push the result.
        if let HeapObj::BoundMethod(recv, id) = self.heap.get(callee) {
            let recv = *recv;
            let id = *id;
            return self.exec_bound_method(recv, id, positional, kw_flat);
        }

        let (fi, captured_defaults, captured_env) = match self.heap.get(callee) {
            HeapObj::Func(i, d, c) => (*i, d.clone(), c.clone()),
            _ => return Err(VmErr::Type("object is not callable")),
        };

        let outer_impure = self.observed_impure.last().copied().unwrap_or(false);
        if num_kw == 0 && !outer_impure
            && let Some(cached) = self.templates.lookup(fi, &positional, &self.heap) {
                self.push(cached);
                return Ok(());
        }

        self.depth += 1;
        let (params, body, _defaults, name_idx) = self.functions[fi];
        let name_idx = *name_idx;

        let mut fn_slots = self.fill_builtins(&body.names);

        let mut body_map: HashMap<&str, usize> =
            HashMap::with_capacity_and_hasher(body.names.len(), Default::default());
        for (i, n) in body.names.iter().enumerate() { body_map.insert(n.as_str(), i); }

        let mut pos_idx = 0usize;
        for param in params.iter() {
            if let Some(star_name) = param.strip_prefix("**") {
                // **kwargs: not yet supported, skip
                let _ = star_name;
            } else if let Some(var_name) = param.strip_prefix('*') {
                // *args: collect all remaining positionals into a list
                let rest: Vec<Val> = positional[pos_idx..].to_vec();
                pos_idx = positional.len();
                let list_val = self.heap.alloc(
                    HeapObj::List(Rc::new(RefCell::new(rest)))
                )?;
                let pname = format!("{}_0", var_name);
                if let Some(&s) = body_map.get(pname.as_str()) {
                    fn_slots[s] = Some(list_val);
                }
            } else {
                if pos_idx >= positional.len() { continue; }
                let pname = format!("{}_0", param);
                if let Some(&s) = body_map.get(pname.as_str()) {
                    fn_slots[s] = Some(positional[pos_idx]);
                }
                pos_idx += 1;
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

        // Apply captured environment from function-creation time (closures).
        // Skip nonlocal variables — they must always come from the live enclosing
        // scope (handled below) so that mutations between calls are visible.
        let nonlocal_body_slots: alloc::collections::BTreeSet<usize> = body.nonlocals.iter()
            .flat_map(|base| {
                body.names.iter().enumerate().filter_map(|(i, n)| {
                    n.rfind('_').filter(|&p| n[p+1..].parse::<u32>().is_ok())
                        .filter(|&p| &n[..p] == base.as_str())
                        .map(|_| i)
                })
            })
            .collect();

        for (bi, val) in &captured_env {
            if nonlocal_body_slots.contains(bi) { continue; }
            if *bi < fn_slots.len() && fn_slots[*bi].is_none() {
                fn_slots[*bi] = Some(*val);
            }
        }

        // Captura del enclosing scope: nombres del frame que llama (`chunk`),
        // valores de sus slots. `.get()` defensivo si las longitudes divergen.
        for (si, sv) in slots.iter().enumerate() {
            if let Some(v) = sv
                && let Some(name) = chunk.names.get(si)
                && let Some(&bs) = body_map.get(name.as_str())
                && fn_slots[bs].is_none()
            {
                fn_slots[bs] = Some(*v);
            }
        }

        // Self-reference: `name_idx` referencia el chunk del CALLER, no el módulo.
        if name_idx != u16::MAX
            && let Some(raw_name) = chunk.names.get(name_idx as usize)
        {
            let base = raw_name.rfind('_')
                .filter(|&p| raw_name[p+1..].parse::<u32>().is_ok())
                .map(|p| &raw_name[..p])
                .unwrap_or(raw_name.as_str());
            let versioned = format!("{}_0", base);
            if let Some(&slot) = body_map.get(versioned.as_str())
                && fn_slots[slot].is_none()
            {
                fn_slots[slot] = Some(callee);
            }
        }

        let yields_before = self.yields.len();
        let snap = self.live_slots.len();
        self.live_slots.extend(slots.iter().flatten().copied());
        self.observed_impure.push(false);

        let exec_result = self.exec(body, &mut fn_slots);

        let callee_impure = self.observed_impure.pop().unwrap_or(true);
        self.live_slots.truncate(snap);
        self.depth -= 1;

        // Write back nonlocal variables to the caller's slots.
        for base in &body.nonlocals {
            // Find the highest-versioned value in the callee's slots.
            let best = body.names.iter().enumerate()
                .filter_map(|(i, n)| {
                    let p = n.rfind('_')?;
                    (n[..p] == **base).then_some(())?;
                    let ver: u32 = n[p+1..].parse().ok()?;
                    Some((ver, fn_slots[i]?))
                })
                .max_by_key(|(ver, _)| *ver)
                .map(|(_, val)| val);

            if let Some(val) = best {
                // Update all versions of this name in the caller's slots.
                for (si, sname) in chunk.names.iter().enumerate() {
                    if let Some(p) = sname.rfind('_')
                        && &sname[..p] == base.as_str() && si < slots.len() {
                            slots[si] = Some(val);
                        }
                }
            }
        }

        let result = exec_result?;
        if callee_impure { self.mark_impure(); }

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
        Ok(())
    }

    /// Bound-method dispatcher. Pure builtins go here; mutation marks
    /// the caller's frame impure so memoization deopts correctly.
    fn exec_bound_method(
        &mut self,
        recv: Val,
        id: crate::modules::vm::types::BuiltinMethodId,
        positional: Vec<Val>,
        kw_flat: Vec<Val>,
    ) -> Result<(), VmErr> {
        use crate::modules::vm::types::BuiltinMethodId::*;

        if !kw_flat.is_empty() {
            return Err(VmErr::Type("builtin method takes no keyword arguments"));
        }

        match id {
            // ─── List Methods ─────────────────────────────────────────────
            ListAppend => {
                expect_args!(positional, 1, "list.append");
                let item = positional[0];
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => rc.borrow_mut().push(item),
                    _ => return Err(VmErr::Type("list.append: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            ListSort => {
                expect_args!(positional, 0, "sort");
                let items = match self.heap.get(recv) {
                    HeapObj::List(rc) => rc.borrow().clone(),
                    _ => return Err(VmErr::Type("sort: receiver is not a list")),
                };
                let mut sorted = items;
                let mut sort_err: Option<VmErr> = None;
                sorted.sort_by(|&a, &b| {
                    if sort_err.is_some() { return core::cmp::Ordering::Equal; }
                    match self.lt_vals(a, b) {
                        Ok(true)  => core::cmp::Ordering::Less,
                        Ok(false) => match self.lt_vals(b, a) {
                            Ok(true)  => core::cmp::Ordering::Greater,
                            Ok(false) => core::cmp::Ordering::Equal,
                            Err(e)    => { sort_err = Some(e); core::cmp::Ordering::Equal }
                        },
                        Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
                    }
                });
                if let Some(e) = sort_err { return Err(e); }
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => *rc.borrow_mut() = sorted,
                    _ => return Err(VmErr::Type("sort: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            ListReverse => {
                expect_args!(positional, 0, "reverse");
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => rc.borrow_mut().reverse(),
                    _ => return Err(VmErr::Type("reverse: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            ListPop => {
                if positional.len() > 1 {
                    return Err(VmErr::Type("pop() takes at most 1 argument(s)"));
                }
                let popped = match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => {
                        let mut b = rc.borrow_mut();
                        if b.is_empty() {
                            return Err(VmErr::Value("pop from empty list"));
                        }
                        if positional.is_empty() {
                            b.pop().unwrap()
                        } else {
                            if !positional[0].is_int() {
                                return Err(VmErr::Type("list indices must be integers"));
                            }
                            let i = positional[0].as_int();
                            let ui = if i < 0 { (b.len() as i64 + i) as usize } else { i as usize };
                            if ui >= b.len() {
                                return Err(VmErr::Value("pop index out of range"));
                            }
                            b.remove(ui)
                        }
                    }
                    _ => return Err(VmErr::Type("pop: receiver is not a list")),
                };
                self.mark_impure();
                self.push(popped);
                Ok(())
            }
            ListInsert => {
                expect_args!(positional, 2, "insert");
                if !positional[0].is_int() {
                    return Err(VmErr::Type("list indices must be integers"));
                }
                let item = positional[1];
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => {
                        let mut b = rc.borrow_mut();
                        let i  = positional[0].as_int();
                        let ui = if i < 0 {
                            (b.len() as i64 + i).max(0) as usize
                        } else {
                            (i as usize).min(b.len())
                        };
                        b.insert(ui, item);
                    }
                    _ => return Err(VmErr::Type("insert: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            ListRemove => {
                expect_args!(positional, 1, "remove");
                let target = positional[0];
                let items = match self.heap.get(recv) {
                    HeapObj::List(rc) => rc.borrow().clone(),
                    _ => return Err(VmErr::Type("remove: receiver is not a list")),
                };
                let pos = items.iter()
                    .position(|&v| eq_vals_with_heap(v, target, &self.heap))
                    .ok_or(VmErr::Value("list.remove: value not found"))?;
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => { rc.borrow_mut().remove(pos); }
                    _ => return Err(VmErr::Type("remove: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            ListIndex => {
                expect_args!(positional, 1, "index");
                let target = positional[0];
                let idx = match self.heap.get(recv) {
                    HeapObj::List(rc) => {
                        rc.borrow().iter().position(|&v| eq_vals_with_heap(v, target, &self.heap))
                            .map(|i| i as i64)
                            .ok_or(VmErr::Value("value not found in list"))?
                    }
                    _ => return Err(VmErr::Type("index: receiver is not a list")),
                };
                self.push(Val::int(idx));
                Ok(())
            }
            ListCount => {
                expect_args!(positional, 1, "count");
                let target = positional[0];
                let n = match self.heap.get(recv) {
                    HeapObj::List(rc) => {
                        rc.borrow().iter().filter(|&&v| eq_vals_with_heap(v, target, &self.heap)).count() as i64
                    }
                    _ => return Err(VmErr::Type("count: receiver is not a list")),
                };
                self.push(Val::int(n));
                Ok(())
            }
            ListExtend => {
                expect_args!(positional, 1, "extend");
                let items: Vec<Val> = if positional[0].is_heap() {
                    match self.heap.get(positional[0]) {
                        HeapObj::List(rc) => rc.borrow().clone(),
                        HeapObj::Tuple(v) => v.clone(),
                        _ => return Err(VmErr::Type("extend() argument must be iterable")),
                    }
                } else { return Err(VmErr::Type("extend() argument must be iterable")); };
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => rc.borrow_mut().extend_from_slice(&items),
                    _ => return Err(VmErr::Type("extend: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none()); Ok(())
            }
            ListClear => {
                expect_args!(positional, 0, "clear");
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => rc.borrow_mut().clear(),
                    _ => return Err(VmErr::Type("clear: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none()); Ok(())
            }
            ListCopy => {
                expect_args!(positional, 0, "copy");
                let items = match self.heap.get(recv) {
                    HeapObj::List(rc) => rc.borrow().clone(),
                    _ => return Err(VmErr::Type("copy: receiver is not a list")),
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
                self.push(val); Ok(())
            }

            // ─── Dictionary Methods ───────────────────────────────────────
            DictKeys => {
                expect_args!(positional, 0, "keys");
                let keys: Vec<Val> = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().keys().collect(),
                    _ => return Err(VmErr::Type("keys: receiver is not a dict")),
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(keys))))?;
                self.push(val);
                Ok(())
            }
            DictValues => {
                expect_args!(positional, 0, "values");
                let values: Vec<Val> = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().entries.iter().map(|&(_, v)| v).collect(),
                    _ => return Err(VmErr::Type("values: receiver is not a dict")),
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(values))))?;
                self.push(val);
                Ok(())
            }
            DictItems => {
                expect_args!(positional, 0, "items");
                let pairs: Vec<(Val, Val)> = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().entries.clone(),
                    _ => return Err(VmErr::Type("items: receiver is not a dict")),
                };
                let mut items: Vec<Val> = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let t = self.heap.alloc(HeapObj::Tuple(vec![k, v]))?;
                    items.push(t);
                }
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
                self.push(val);
                Ok(())
            }
            DictGet => {
                if positional.is_empty() || positional.len() > 2 {
                    return Err(VmErr::Type("get() takes 1 or 2 argument(s)"));
                }
                let key      = positional[0];
                let default  = if positional.len() == 2 { positional[1] } else { Val::none() };
                let result = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().get(&key).copied().unwrap_or(default),
                    _ => return Err(VmErr::Type("get: receiver is not a dict")),
                };
                self.push(result);
                Ok(())
            }
            DictUpdate => {
                expect_args!(positional, 1, "update");
                let pairs = match self.heap.get(positional[0]) {
                    HeapObj::Dict(rc) => rc.borrow().entries.clone(),
                    _ => return Err(VmErr::Type("update() argument must be a dict")),
                };
                match self.heap.get_mut(recv) {
                    HeapObj::Dict(rc) => {
                        let mut b = rc.borrow_mut();
                        for (k, v) in pairs { b.insert(k, v); }
                    }
                    _ => return Err(VmErr::Type("update: receiver is not a dict")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            DictPop => {
                if positional.is_empty() || positional.len() > 2 {
                    return Err(VmErr::Type("pop() takes 1 or 2 argument(s)"));
                }
                let key     = positional[0];
                let default = if positional.len() == 2 { Some(positional[1]) } else { None };
                let result = match self.heap.get_mut(recv) {
                    HeapObj::Dict(rc) => {
                        let mut b = rc.borrow_mut();
                        if let Some(val) = b.remove(&key) {
                            val
                        } else {
                            match default {
                                Some(d) => d,
                                None    => return Err(VmErr::Value("key not found")),
                            }
                        }
                    }
                    _ => return Err(VmErr::Type("pop: receiver is not a dict")),
                };
                self.mark_impure();
                self.push(result);
                Ok(())
            }
            DictSetDefault => {
                if positional.is_empty() {
                    return Err(VmErr::Type("setdefault() requires at least 1 argument(s)"));
                }
                let key = positional[0];
                let default = if positional.len() > 1 { positional[1] } else { Val::none() };
                let result = match self.heap.get_mut(recv) {
                    HeapObj::Dict(rc) => {
                        let already = rc.borrow().get(&key).copied();
                        if let Some(v) = already { v } else {
                            rc.borrow_mut().insert(key, default);
                            default
                        }
                    }
                    _ => return Err(VmErr::Type("setdefault: receiver is not a dict")),
                };
                self.mark_impure();
                self.push(result);
                Ok(())
            }

            // ─── String Methods (using macros) ────────────────────────────
            StrUpper => str_no_args!(self, recv, positional, "upper", |s: String| s.to_uppercase()),
            StrLower => str_no_args!(self, recv, positional, "lower", |s: String| s.to_lowercase()),
            StrStrip => str_no_args!(self, recv, positional, "strip", |s: String| s.trim().to_string()),
            
            StrCapitalize => str_no_args!(self, recv, positional, "capitalize", |s: String| {
                let mut cs = s.chars();
                cs.next().map(|c| c.to_uppercase().to_string() + cs.as_str().to_lowercase().as_str())
                    .unwrap_or_default()
            }),

            StrStartswith => str_one_str_arg!(self, recv, positional, "startswith", |s: String, arg: String| {
                Val::bool(s.starts_with(arg.as_str()))
            }),
            StrEndswith => str_one_str_arg!(self, recv, positional, "endswith", |s: String, arg: String| {
                Val::bool(s.ends_with(arg.as_str()))
            }),
            StrFind => str_one_str_arg!(self, recv, positional, "find", |s: String, sub: String| {
                let idx = s.find(sub.as_str())
                    .map(|i| s[..i].chars().count() as i64)
                    .unwrap_or(-1);
                Val::int(idx)
            }),
            StrCount => str_one_str_arg!(self, recv, positional, "count", |s: String, sub: String| {
                Val::int(s.matches(sub.as_str()).count() as i64)
            }),

            // ─── String Methods (mixed arguments) ─────────────────────────
            StrJoin => {
                expect_args!(positional, 1, "join");
                let sep = self.recv_str(recv)?;
                let items = match self.heap.get(positional[0]) {
                    HeapObj::List(rc)  => rc.borrow().clone(),
                    HeapObj::Tuple(v)  => v.clone(),
                    _ => return Err(VmErr::Type("join() argument must be iterable")),
                };
                let mut parts: Vec<String> = Vec::with_capacity(items.len());
                for v in items {
                    parts.push(self.val_to_str(v)?);
                }
                let val = self.heap.alloc(HeapObj::Str(parts.join(sep.as_str())))?;
                self.push(val);
                Ok(())
            }
            StrReplace => {
                expect_args!(positional, 2, "replace");
                let s   = self.recv_str(recv)?;
                let old = self.val_to_str(positional[0])?;
                let new = self.val_to_str(positional[1])?;
                let val = self.heap.alloc(HeapObj::Str(s.replace(old.as_str(), new.as_str())))?;
                self.push(val);
                Ok(())
            }
            StrSplit => {
                let s = self.recv_str(recv)?;
                let parts: Vec<Val> = if positional.is_empty() {
                    s.split_whitespace()
                        .map(|p| self.heap.alloc(HeapObj::Str(p.to_string())))
                        .collect::<Result<_, _>>()?
                } else {
                    let sep = self.val_to_str(positional[0])?;
                    s.split(sep.as_str())
                        .map(|p| self.heap.alloc(HeapObj::Str(p.to_string())))
                        .collect::<Result<_, _>>()?
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(parts))))?;
                self.push(val);
                Ok(())
            }
            StrLstrip => {
                let s = self.recv_str(recv)?;
                let result = if positional.is_empty() { s.trim_start().to_string() }
                    else { let p = self.val_to_str(positional[0])?; s.trim_start_matches(|c| p.contains(c)).to_string() };
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val); Ok(())
            }
            StrRstrip => {
                let s = self.recv_str(recv)?;
                let result = if positional.is_empty() { s.trim_end().to_string() }
                    else { let p = self.val_to_str(positional[0])?; s.trim_end_matches(|c| p.contains(c)).to_string() };
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val); Ok(())
            }
            StrIsDigit => {
                expect_args!(positional, 0, "isdigit");
                let s = self.recv_str(recv)?;
                self.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())));
                Ok(())
            }
            StrIsAlpha => {
                expect_args!(positional, 0, "isalpha");
                let s = self.recv_str(recv)?;
                self.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())));
                Ok(())
            }
            StrIsAlnum => {
                expect_args!(positional, 0, "isalnum");
                let s = self.recv_str(recv)?;
                self.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())));
                Ok(())
            }
            StrTitle => {
                expect_args!(positional, 0, "title");
                let s = self.recv_str(recv)?;
                let result = s.split_whitespace()
                    .map(|w| { let mut cs = w.chars(); cs.next().map(|c| c.to_uppercase().to_string() + cs.as_str()).unwrap_or_default() })
                    .collect::<Vec<_>>().join(" ");
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val); Ok(())
            }
            StrCenter => {
                if positional.is_empty() { return Err(VmErr::Type("center() requires at least 1 argument(s)")); }
                let s = self.recv_str(recv)?;
                if !positional[0].is_int() { return Err(VmErr::Type("center() width must be an integer")); }
                let width = positional[0].as_int() as usize;
                let fill = if positional.len() > 1 { self.val_to_str(positional[1])?.chars().next().unwrap_or(' ') } else { ' ' };
                let pad = width.saturating_sub(s.len());
                let left = pad / 2;
                let right = pad - left;
                let result = fill.to_string().repeat(left) + &s + &fill.to_string().repeat(right);
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val); Ok(())
            }
            StrZfill => {
                if positional.is_empty() || !positional[0].is_int() {
                    return Err(VmErr::Type("zfill() requires an integer argument"));
                }
                let s = self.recv_str(recv)?;
                let width = positional[0].as_int() as usize;
                let result = if s.len() >= width { s } else {
                    let pad = "0".repeat(width - s.len());
                    if s.starts_with('+') || s.starts_with('-') {
                        s[..1].to_string() + &pad + &s[1..]
                    } else { pad + &s }
                };
                let val = self.heap.alloc(HeapObj::Str(result))?;
                self.push(val); Ok(())
            }
        }
    }

    // Extracts the string content from a Str receiver.
    fn recv_str(&self, recv: Val) -> Result<String, VmErr> {
        match self.heap.get(recv) {
            HeapObj::Str(s) => Ok(s.clone()),
            _ => Err(VmErr::Type("method requires a string receiver")),
        }
    }

    // Converts a Val to String for use as a method argument.
    fn val_to_str(&self, v: Val) -> Result<String, VmErr> {
        match self.heap.get(v) {
            HeapObj::Str(s) => Ok(s.clone()),
            _ => Err(VmErr::Type("argument must be a string")),
        }
    }
}