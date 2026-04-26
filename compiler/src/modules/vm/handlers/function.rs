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
        }
    }
}