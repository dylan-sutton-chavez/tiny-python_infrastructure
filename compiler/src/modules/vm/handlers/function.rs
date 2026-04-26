// vm/handlers/function.rs

use super::*;

impl<'a> VM<'a> {
    pub(crate) fn handle_function(
        &mut self, op: OpCode, operand: u16,
        chunk: &SSAChunk, slots: &mut [Option<Val>]
    ) -> Result<(), VmErr> {
        match op {
            // User functions
            OpCode::Call => self.exec_call(operand, chunk, slots),
            OpCode::MakeFunction | OpCode::MakeCoroutine => self.exec_make_function(operand, chunk),

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

    fn exec_make_function(&mut self, operand: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        // `operand` is the local index in `chunk.functions`. Translate to the
        // global id assigned during VM init so the resulting Func resolves
        // correctly even after escaping its defining scope.
        let global = self.fn_index
            .get(&(chunk as *const _))
            .and_then(|v| v.get(operand as usize).copied())
            .ok_or(VmErr::Runtime("MakeFunction: unknown function index"))?;
        let n_defaults = self.functions[global as usize].2 as usize;
        let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };
        let val = self.heap.alloc(HeapObj::Func(global as usize, defaults))?;
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

        let (fi, captured_defaults) = match self.heap.get(callee) {
            HeapObj::Func(i, d) => (*i, d.clone()),
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

        // Capturar el Result; cleanup corre incondicionalmente antes de propagar Err.
        let exec_result = self.exec(body, &mut fn_slots);

        let callee_impure = self.observed_impure.pop().unwrap_or(true);
        self.live_slots.truncate(snap);
        self.depth -= 1;

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
            ListAppend => {
                if positional.len() != 1 {
                    return Err(VmErr::Type("list.append() takes exactly one argument"));
                }
                let item = positional[0];
                match self.heap.get_mut(recv) {
                    HeapObj::List(rc) => rc.borrow_mut().push(item),
                    _ => return Err(VmErr::Type("list.append: receiver is not a list")),
                }
                self.mark_impure();
                self.push(Val::none());
                Ok(())
            }
            DictKeys => {
                if !positional.is_empty() {
                    return Err(VmErr::Type("keys() takes no arguments"));
                }
                let keys: Vec<Val> = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().keys().collect(),
                    _ => return Err(VmErr::Type("keys: receiver is not a dict")),
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(keys))))?;
                self.push(val);
                Ok(())
            }
            DictValues => {
                if !positional.is_empty() {
                    return Err(VmErr::Type("values() takes no arguments"));
                }
                let values: Vec<Val> = match self.heap.get(recv) {
                    HeapObj::Dict(rc) => rc.borrow().entries.iter().map(|&(_, v)| v).collect(),
                    _ => return Err(VmErr::Type("values: receiver is not a dict")),
                };
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(values))))?;
                self.push(val);
                Ok(())
            }
            DictItems => {
                if !positional.is_empty() {
                    return Err(VmErr::Type("items() takes no arguments"));
                }
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
            StrUpper => {
                let s = self.recv_str(recv)?;
                let val = self.heap.alloc(HeapObj::Str(s.to_uppercase()))?;
                self.push(val);
                Ok(())
            }
            StrLower => {
                let s = self.recv_str(recv)?;
                let val = self.heap.alloc(HeapObj::Str(s.to_lowercase()))?;
                self.push(val);
                Ok(())
            }
            StrStrip => {
                let s = self.recv_str(recv)?;
                let val = self.heap.alloc(HeapObj::Str(s.trim().to_string()))?;
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
            StrJoin => {
                let sep = self.recv_str(recv)?;
                if positional.len() != 1 {
                    return Err(VmErr::Type("join() takes exactly one argument"));
                }
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
                if positional.len() != 2 {
                    return Err(VmErr::Type("replace() takes exactly 2 arguments"));
                }
                let s   = self.recv_str(recv)?;
                let old = self.val_to_str(positional[0])?;
                let new = self.val_to_str(positional[1])?;
                let val = self.heap.alloc(HeapObj::Str(s.replace(old.as_str(), new.as_str())))?;
                self.push(val);
                Ok(())
            }
            StrStartswith => {
                if positional.len() != 1 {
                    return Err(VmErr::Type("startswith() takes exactly one argument"));
                }
                let s      = self.recv_str(recv)?;
                let prefix = self.val_to_str(positional[0])?;
                self.push(Val::bool(s.starts_with(prefix.as_str())));
                Ok(())
            }
            StrEndswith => {
                if positional.len() != 1 {
                    return Err(VmErr::Type("endswith() takes exactly one argument"));
                }
                let s      = self.recv_str(recv)?;
                let suffix = self.val_to_str(positional[0])?;
                self.push(Val::bool(s.ends_with(suffix.as_str())));
                Ok(())
            }
            StrFind => {
                if positional.len() != 1 {
                    return Err(VmErr::Type("find() takes exactly one argument"));
                }
                let s   = self.recv_str(recv)?;
                let sub = self.val_to_str(positional[0])?;
                let idx = s.find(sub.as_str())
                    .map(|i| s[..i].chars().count() as i64)
                    .unwrap_or(-1);
                self.push(Val::int(idx));
                Ok(())
            }
            StrCount => {
                if positional.len() != 1 {
                    return Err(VmErr::Type("count() takes exactly one argument"));
                }
                let s   = self.recv_str(recv)?;
                let sub = self.val_to_str(positional[0])?;
                let n   = s.matches(sub.as_str()).count() as i64;
                self.push(Val::int(n));
                Ok(())
            }
            ListSort => {
                if !positional.is_empty() {
                    return Err(VmErr::Type("sort() takes no arguments"));
                }
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
                if !positional.is_empty() {
                    return Err(VmErr::Type("reverse() takes no arguments"));
                }
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
                    return Err(VmErr::Type("pop() takes at most one argument"));
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
                if positional.len() != 2 {
                    return Err(VmErr::Type("insert() takes exactly 2 arguments"));
                }
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
                if positional.len() != 1 {
                    return Err(VmErr::Type("remove() takes exactly one argument"));
                }
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
                if positional.len() != 1 {
                    return Err(VmErr::Type("index() takes exactly one argument"));
                }
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
                if positional.len() != 1 {
                    return Err(VmErr::Type("count() takes exactly one argument"));
                }
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