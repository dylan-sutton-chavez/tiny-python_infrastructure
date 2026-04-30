// vm/handlers/function.rs

use crate::s;
use super::*;

impl<'a> VM<'a> {
    pub(crate) fn handle_function(
        &mut self, op: OpCode, operand: u16,
        chunk: &SSAChunk, slots: &mut [Option<Val>]
    ) -> Result<(), VmErr> {
        match op {
            OpCode::Call => self.exec_call(operand, chunk, slots),
            OpCode::MakeFunction | OpCode::MakeCoroutine => self.exec_make_function(operand, chunk, slots),
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
            OpCode::CallPrint => { self.mark_impure(); self.call_print(operand) }
            OpCode::CallInput => { self.mark_impure(); self.call_input() }
            OpCode::CallAll      => self.call_all(operand),
            OpCode::CallAny      => self.call_any(operand),
            OpCode::CallBin      => self.call_bin(),
            OpCode::CallOct      => self.call_oct(),
            OpCode::CallHex      => self.call_hex(),
            OpCode::CallDivmod   => self.call_divmod(),
            OpCode::CallPow      => self.call_pow(operand),
            OpCode::CallRepr     => self.call_repr(),
            OpCode::CallReversed => self.call_reversed(),
            OpCode::CallCallable => self.call_callable(),
            OpCode::CallId       => self.call_id(),
            OpCode::CallHash     => self.call_hash(),
            _ => unreachable!("non-function opcode in handle_function"),
        }
    }

    pub(crate) fn exec_bound_method(
        &mut self, recv: Val,
        id: super::methods::BuiltinMethodId,
        pos: Vec<Val>, kw: Vec<Val>,
    ) -> Result<(), VmErr> {
        super::methods::dispatch_method(self, id, recv, pos, kw)
    }

    fn exec_make_function(&mut self, operand: u16, chunk: &SSAChunk, slots: &[Option<Val>]) -> Result<(), VmErr> {
        let global = self.fn_index
            .get(&(chunk as *const _))
            .and_then(|v| v.get(operand as usize).copied())
            .ok_or(cold_runtime("MakeFunction: unknown function index"))? as usize;

        let n_defaults = self.functions[global].2 as usize;
        let defaults = if n_defaults > 0 { self.pop_n(n_defaults)? } else { vec![] };

        let chunk_map: HashMap<&str, usize> = chunk.names.iter()
            .enumerate().map(|(i, n)| (n.as_str(), i)).collect();

        let (params, body, _, _) = self.functions[global];
        let param_names: alloc::collections::BTreeSet<String> = params.iter().map(|p| s!(str p.trim_start_matches('*'), "_0")).collect();
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

        let base_pos = (raw & 0xFF)        as i32;
        let base_kw  = ((raw >> 8) & 0xFF) as i32;
        let num_pos = (base_pos + self.pending_pos_delta).max(0) as usize;
        let num_kw  = (base_kw  + self.pending_kw_delta ).max(0) as usize;
        self.pending_pos_delta = 0;
        self.pending_kw_delta  = 0;

        let total_items = num_pos + 2 * num_kw;

        if self.depth >= self.max_calls { return Err(cold_depth()); }

        let mut stack_items: Vec<Val> = (0..total_items)
            .map(|_| self.pop())
            .collect::<Result<_, _>>()?;
        stack_items.reverse();

        let kw_flat: Vec<Val> = stack_items.split_off(num_pos);
        let positional = stack_items;

        let callee = self.pop()?;
        if !callee.is_heap() { return Err(cold_type("object is not callable")); }

        if let HeapObj::BoundMethod(recv, id) = self.heap.get(callee) {
            let recv = *recv;
            let id = *id;
            return self.exec_bound_method(recv, id, positional, kw_flat);
        }

        if let HeapObj::NativeFn(id) = self.heap.get(callee) {
            let id = *id;
            return self.dispatch_native(id, positional, kw_flat);
        }

        let (fi, captured_defaults, captured_env) = match self.heap.get(callee) {
            HeapObj::Func(i, d, c) => (*i, d.clone(), c.clone()),
            _ => return Err(cold_type("object is not callable")),
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
                let _ = star_name;
            } else if let Some(var_name) = param.strip_prefix('*') {
                let rest: Vec<Val> = positional[pos_idx..].to_vec();
                pos_idx = positional.len();
                let list_val = self.heap.alloc(
                    HeapObj::List(Rc::new(RefCell::new(rest)))
                )?;
                let pname = s!(str var_name, "_0");
                if let Some(&s) = body_map.get(pname.as_str()) {
                    fn_slots[s] = Some(list_val);
                }
            } else {
                if pos_idx >= positional.len() { continue; }
                let pname = s!(str param, "_0");
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
                _ => return Err(cold_runtime("malformed kwarg on stack")),
            };
            if params.iter().any(|p| p.trim_start_matches('*') == key.as_str()) {
                let pname = s!(str &key, "_0");
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
                    let pname = s!(str param.trim_start_matches('*'), "_0");
                    if let Some(&s) = body_map.get(pname.as_str())
                        && fn_slots[s].is_none()
                    {
                        fn_slots[s] = Some(dv);
                    }
                }
            }
        }

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

        for (si, sv) in slots.iter().enumerate() {
            if let Some(v) = sv
                && let Some(name) = chunk.names.get(si)
                && let Some(&bs) = body_map.get(name.as_str())
                && fn_slots[bs].is_none()
            {
                fn_slots[bs] = Some(*v);
            }
        }

        if name_idx != u16::MAX
            && let Some(raw_name) = chunk.names.get(name_idx as usize)
        {
            let base = raw_name.rfind('_')
                .filter(|&p| raw_name[p+1..].parse::<u32>().is_ok())
                .map(|p| &raw_name[..p])
                .unwrap_or(raw_name.as_str());
            let versioned = s!(str base, "_0");
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

        for base in &body.nonlocals {
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

    pub(crate) fn dispatch_native(
        &mut self, id: super::super::types::NativeFnId,
        positional: Vec<Val>, kw: Vec<Val>,
    ) -> Result<(), VmErr> {
        if !kw.is_empty() {
            return Err(cold_type("native function takes no keyword arguments"));
        }
        let argc = positional.len() as u16;

        use super::super::types::NativeFnId::*;

        // Pre-validate fixed arity to keep the stack clean on error.
        let expected: Option<u16> = match id {
            Input => Some(0),
            Len | Abs | Str | Int | Float | Bool | Type | Chr | Ord
            | Sorted | Enumerate | List | Tuple | Bin | Oct | Hex
            | Repr | Reversed | Callable | Id | Hash | Ascii => Some(1),
            Divmod | IsInstance | HasAttr => Some(2),
            _ => None,
        };
        if let Some(n) = expected
            && argc != n {
                return Err(cold_type("wrong number of arguments to builtin"));
        }

        for v in positional { self.push(v); }

        match id {
            // Variadic
            Print => {
                // call_print is statement-shaped: the dedicated CallPrint opcode
                // is emitted by the parser without a trailing Pop. When dispatched
                // indirectly via Call (e.g. `p = print; p(42)`), the parser does
                // emit a Pop to discard the expression-statement value, so we
                // must materialize Python's implicit `None` return here to keep
                // the stack balanced.
                self.call_print(argc)?;
                self.push(Val::none());
                Ok(())
            }
            Range => self.call_range(argc),
            Round => self.call_round(argc),
            Min => self.call_min(argc),
            Max => self.call_max(argc),
            Sum => self.call_sum(argc),
            Zip => self.call_zip(argc),
            Dict => self.call_dict(argc),
            Set => self.call_set(argc),
            Pow => self.call_pow(argc),
            All => self.call_all(argc),
            Any => self.call_any(argc),
            GetAttr => self.call_getattr(argc),
            Format => self.call_format(argc),
            // 0/1/2-arg
            Input => self.call_input(),
            Len => self.call_len(),
            Abs => self.call_abs(),
            Str => self.call_str(),
            Int => self.call_int(),
            Float => self.call_float(),
            Bool => self.call_bool(),
            Type => self.call_type(),
            Chr => self.call_chr(),
            Ord => self.call_ord(),
            Sorted => self.call_sorted(),
            Enumerate => self.call_enumerate(),
            List => self.call_list(),
            Tuple => self.call_tuple(),
            Bin => self.call_bin(),
            Oct => self.call_oct(),
            Hex => self.call_hex(),
            Repr => self.call_repr(),
            Reversed => self.call_reversed(),
            Callable => self.call_callable(),
            Id => self.call_id(),
            Hash => self.call_hash(),
            Ascii => self.call_ascii(),
            Divmod => self.call_divmod(),
            IsInstance => self.call_isinstance(),
            HasAttr => self.call_hasattr(),
        }
    }
}