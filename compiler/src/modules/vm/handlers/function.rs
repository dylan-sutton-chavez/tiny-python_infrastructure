// vm/handlers/function.rs

use crate::s;
use crate::alloc::string::ToString;

use super::*;

macro_rules! str_no_args {
    ($vm:ident, $recv:ident, $pos:ident, $method:literal, $transform:expr) => {{
        expect_args!($pos, 0, $method);
        let s   = $vm.recv_str($recv)?;
        let out = $transform(s);
        let val = $vm.heap.alloc(HeapObj::Str(out))?;
        $vm.push(val);
        Ok(())
    }};
}

macro_rules! expect_args {
    ($pos:ident, $n:expr, $name:literal) => {
        if $pos.len() != $n {
            return Err(cold_type(concat!(
                $name, "() takes exactly ", stringify!($n), " argument(s)"
            )));
        }
    };
}

macro_rules! str_one_str_arg {
    ($vm:ident, $recv:ident, $pos:ident, $name:literal, $body:expr) => {{
        expect_args!($pos, 1, $name);
        let s   = $vm.recv_str($recv)?;
        let arg = $vm.val_to_str($pos[0])?;
        let result = $body(s, arg);
        $vm.push(result);
        Ok(())
    }};
}

type MethodHandler = fn(&mut VM, Val, Vec<Val>, Vec<Val>) -> Result<(), VmErr>;

static METHOD_TABLE: &[MethodHandler] = &[
    method_list_append,     // 0
    method_dict_keys,       // 1
    method_dict_values,     // 2
    method_dict_items,      // 3
    method_str_upper,       // 4
    method_str_lower,       // 5
    method_str_strip,       // 6
    method_str_split,       // 7
    method_str_join,        // 8
    method_str_replace,     // 9
    method_str_startswith,  // 10
    method_str_endswith,    // 11
    method_str_find,        // 12
    method_str_count,       // 13
    method_list_sort,       // 14
    method_list_reverse,    // 15
    method_list_pop,        // 16
    method_list_insert,     // 17
    method_list_remove,     // 18
    method_list_index,      // 19
    method_list_count,      // 20
    method_dict_get,        // 21
    method_dict_update,     // 22
    method_dict_pop,        // 23
    method_dict_setdefault, // 24
    method_str_lstrip,      // 25
    method_str_rstrip,      // 26
    method_str_isdigit,     // 27
    method_str_isalpha,     // 28
    method_str_isalnum,     // 29
    method_str_capitalize,  // 30
    method_str_title,       // 31
    method_str_center,      // 32
    method_str_zfill,       // 33
    method_list_extend,     // 34
    method_list_clear,      // 35
    method_list_copy,       // 36
];


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
            _ => unreachable!("non-function opcode in handle_function"),
        }
    }

    pub(crate) fn exec_bound_method(
        &mut self, recv: Val,
        id: crate::modules::vm::types::BuiltinMethodId,
        pos: Vec<Val>, kw: Vec<Val>,
    ) -> Result<(), VmErr> {
        if !kw.is_empty() { return Err(cold_type("builtin method takes no keyword arguments")); }
        METHOD_TABLE[id as usize](self, recv, pos, kw)
    }

    fn recv_str(&self, recv: Val) -> Result<String, VmErr> {
        match self.heap.get(recv) {
            HeapObj::Str(s) => Ok(s.clone()),
            _ => Err(cold_type("method requires a string receiver")),
        }
    }

    fn val_to_str(&self, v: Val) -> Result<String, VmErr> {
        match self.heap.get(v) {
            HeapObj::Str(s) => Ok(s.clone()),
            _ => Err(cold_type("argument must be a string")),
        }
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

        // Aplica y consume los deltas dejados por UnpackArgs antes de este Call.
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
}

fn method_list_append(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "list.append");
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => rc.borrow_mut().push(pos[0]),
        _ => return Err(cold_type("list.append: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_sort(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "sort");
    let mut sorted = match vm.heap.get(recv) {
        HeapObj::List(rc) => rc.borrow().clone(),
        _ => return Err(cold_type("sort: receiver is not a list")),
    };
    let mut sort_err: Option<VmErr> = None;
    sorted.sort_by(|&a, &b| {
        if sort_err.is_some() { return core::cmp::Ordering::Equal; }
        match vm.lt_vals(a, b) {
            Ok(true) => core::cmp::Ordering::Less,
            Ok(false) => match vm.lt_vals(b, a) {
                Ok(true) => core::cmp::Ordering::Greater,
                Ok(false) => core::cmp::Ordering::Equal,
                Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
            },
            Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
        }
    });
    if let Some(e) = sort_err { return Err(e); }
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => *rc.borrow_mut() = sorted,
        _ => return Err(cold_type("sort: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_reverse(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "reverse");
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => rc.borrow_mut().reverse(),
        _ => return Err(cold_type("reverse: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_pop(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.len() > 1 { return Err(cold_type("pop() takes at most 1 argument(s)")); }
    let popped = match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => {
            let mut b = rc.borrow_mut();
            if b.is_empty() { return Err(cold_value("pop from empty list")); }
            if pos.is_empty() { b.pop().unwrap() }
            else {
                if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
                let i = pos[0].as_int();
                let ui = if i < 0 { (b.len() as i64 + i) as usize } else { i as usize };
                if ui >= b.len() { return Err(cold_value("pop index out of range")); }
                b.remove(ui)
            }
        }
        _ => return Err(cold_type("pop: receiver is not a list")),
    };
    vm.mark_impure(); vm.push(popped); Ok(())
}

fn method_list_insert(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 2, "insert");
    if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => {
            let mut b = rc.borrow_mut();
            let i = pos[0].as_int();
            let ui = if i < 0 { (b.len() as i64 + i).max(0) as usize } else { (i as usize).min(b.len()) };
            b.insert(ui, pos[1]);
        }
        _ => return Err(cold_type("insert: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_remove(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "remove");
    let items = match vm.heap.get(recv) {
        HeapObj::List(rc) => rc.borrow().clone(),
        _ => return Err(cold_type("remove: receiver is not a list")),
    };
    let idx = items.iter()
        .position(|&v| eq_vals_with_heap(v, pos[0], &vm.heap))
        .ok_or(cold_value("list.remove: value not found"))?;
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => { rc.borrow_mut().remove(idx); }
        _ => return Err(cold_type("remove: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_index(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "index");
    let idx = match vm.heap.get(recv) {
        HeapObj::List(rc) => rc.borrow().iter().position(|&v| eq_vals_with_heap(v, pos[0], &vm.heap))
            .map(|i| i as i64).ok_or(cold_value("value not found in list"))?,
        _ => return Err(cold_type("index: receiver is not a list")),
    };
    vm.push(Val::int(idx)); Ok(())
}

fn method_list_count(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "count");
    let n = match vm.heap.get(recv) {
        HeapObj::List(rc) => rc.borrow().iter().filter(|&&v| eq_vals_with_heap(v, pos[0], &vm.heap)).count() as i64,
        _ => return Err(cold_type("count: receiver is not a list")),
    };
    vm.push(Val::int(n)); Ok(())
}

fn method_list_extend(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "extend");
    let items: Vec<Val> = if pos[0].is_heap() {
        match vm.heap.get(pos[0]) {
            HeapObj::List(rc) => rc.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            _ => return Err(cold_type("extend() argument must be iterable")),
        }
    } else { return Err(cold_type("extend() argument must be iterable")); };
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => rc.borrow_mut().extend_from_slice(&items),
        _ => return Err(cold_type("extend: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_clear(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "clear");
    match vm.heap.get_mut(recv) {
        HeapObj::List(rc) => rc.borrow_mut().clear(),
        _ => return Err(cold_type("clear: receiver is not a list")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_list_copy(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "copy");
    let items = match vm.heap.get(recv) {
        HeapObj::List(rc) => rc.borrow().clone(),
        _ => return Err(cold_type("copy: receiver is not a list")),
    };
    let val = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
    vm.push(val); Ok(())
}

fn method_dict_keys(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "keys");
    let keys: Vec<Val> = match vm.heap.get(recv) {
        HeapObj::Dict(rc) => rc.borrow().keys().collect(),
        _ => return Err(cold_type("keys: receiver is not a dict")),
    };
    let val = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(keys))))?;
    vm.push(val); Ok(())
}

fn method_dict_values(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "values");
    let values: Vec<Val> = match vm.heap.get(recv) {
        HeapObj::Dict(rc) => rc.borrow().entries.iter().map(|&(_, v)| v).collect(),
        _ => return Err(cold_type("values: receiver is not a dict")),
    };
    let val = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(values))))?;
    vm.push(val); Ok(())
}

fn method_dict_items(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "items");
    let pairs: Vec<(Val, Val)> = match vm.heap.get(recv) {
        HeapObj::Dict(rc) => rc.borrow().entries.clone(),
        _ => return Err(cold_type("items: receiver is not a dict")),
    };
    let mut items: Vec<Val> = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let t = vm.heap.alloc(HeapObj::Tuple(vec![k, v]))?;
        items.push(t);
    }
    let val = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
    vm.push(val); Ok(())
}

fn method_dict_get(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.is_empty() || pos.len() > 2 { return Err(cold_type("get() takes 1 or 2 argument(s)")); }
    let default = if pos.len() == 2 { pos[1] } else { Val::none() };
    let result = match vm.heap.get(recv) {
        HeapObj::Dict(rc) => rc.borrow().get(&pos[0]).copied().unwrap_or(default),
        _ => return Err(cold_type("get: receiver is not a dict")),
    };
    vm.push(result); Ok(())
}

fn method_dict_update(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "update");
    let pairs = match vm.heap.get(pos[0]) {
        HeapObj::Dict(rc) => rc.borrow().entries.clone(),
        _ => return Err(cold_type("update() argument must be a dict")),
    };
    match vm.heap.get_mut(recv) {
        HeapObj::Dict(rc) => { let mut b = rc.borrow_mut(); for (k, v) in pairs { b.insert(k, v); } }
        _ => return Err(cold_type("update: receiver is not a dict")),
    }
    vm.mark_impure(); vm.push(Val::none()); Ok(())
}

fn method_dict_pop(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.is_empty() || pos.len() > 2 { return Err(cold_type("pop() takes 1 or 2 argument(s)")); }
    let default = if pos.len() == 2 { Some(pos[1]) } else { None };
    let result = match vm.heap.get_mut(recv) {
        HeapObj::Dict(rc) => {
            let mut b = rc.borrow_mut();
            if let Some(val) = b.remove(&pos[0]) { val }
            else { match default { Some(d) => d, None => return Err(cold_value("key not found")) } }
        }
        _ => return Err(cold_type("pop: receiver is not a dict")),
    };
    vm.mark_impure(); vm.push(result); Ok(())
}

fn method_dict_setdefault(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.is_empty() { return Err(cold_type("setdefault() requires at least 1 argument(s)")); }
    let default = if pos.len() > 1 { pos[1] } else { Val::none() };
    let result = match vm.heap.get_mut(recv) {
        HeapObj::Dict(rc) => {
            let already = rc.borrow().get(&pos[0]).copied();
            if let Some(v) = already { v } else { rc.borrow_mut().insert(pos[0], default); default }
        }
        _ => return Err(cold_type("setdefault: receiver is not a dict")),
    };
    vm.mark_impure(); vm.push(result); Ok(())
}

fn method_str_upper(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_no_args!(vm, recv, pos, "upper", |s: String| s.to_uppercase())
}
fn method_str_lower(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_no_args!(vm, recv, pos, "lower", |s: String| s.to_lowercase())
}
fn method_str_strip(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_no_args!(vm, recv, pos, "strip", |s: String| s.trim().to_string())
}
fn method_str_capitalize(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_no_args!(vm, recv, pos, "capitalize", |s: String| {
        let mut cs = s.chars();
        cs.next().map(|c| c.to_uppercase().to_string() + cs.as_str().to_lowercase().as_str())
            .unwrap_or_default()
    })
}
fn method_str_startswith(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_one_str_arg!(vm, recv, pos, "startswith", |s: String, arg: String| Val::bool(s.starts_with(arg.as_str())))
}
fn method_str_endswith(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_one_str_arg!(vm, recv, pos, "endswith", |s: String, arg: String| Val::bool(s.ends_with(arg.as_str())))
}
fn method_str_find(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_one_str_arg!(vm, recv, pos, "find", |s: String, sub: String| {
        Val::int(s.find(sub.as_str()).map(|i| s[..i].chars().count() as i64).unwrap_or(-1))
    })
}
fn method_str_count(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    str_one_str_arg!(vm, recv, pos, "count", |s: String, sub: String| Val::int(s.matches(sub.as_str()).count() as i64))
}
fn method_str_join(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 1, "join");
    let sep = vm.recv_str(recv)?;
    let items = match vm.heap.get(pos[0]) {
        HeapObj::List(rc) => rc.borrow().clone(),
        HeapObj::Tuple(v) => v.clone(),
        _ => return Err(cold_type("join() argument must be iterable")),
    };
    let mut parts: Vec<String> = Vec::with_capacity(items.len());
    for v in items { parts.push(vm.val_to_str(v)?); }
    let val = vm.heap.alloc(HeapObj::Str(parts.join(sep.as_str())))?;
    vm.push(val); Ok(())
}
fn method_str_replace(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 2, "replace");
    let s = vm.recv_str(recv)?;
    let old = vm.val_to_str(pos[0])?;
    let new = vm.val_to_str(pos[1])?;
    let val = vm.heap.alloc(HeapObj::Str(s.replace(old.as_str(), new.as_str())))?;
    vm.push(val); Ok(())
}
fn method_str_split(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    let s = vm.recv_str(recv)?;
    let parts: Vec<Val> = if pos.is_empty() {
        s.split_whitespace().map(|p| vm.heap.alloc(HeapObj::Str(p.to_string()))).collect::<Result<_, _>>()?
    } else {
        let sep = vm.val_to_str(pos[0])?;
        s.split(sep.as_str()).map(|p| vm.heap.alloc(HeapObj::Str(p.to_string()))).collect::<Result<_, _>>()?
    };
    let val = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(parts))))?;
    vm.push(val); Ok(())
}
fn method_str_lstrip(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    let s = vm.recv_str(recv)?;
    let result = if pos.is_empty() { s.trim_start().to_string() }
        else { let p = vm.val_to_str(pos[0])?; s.trim_start_matches(|c| p.contains(c)).to_string() };
    let val = vm.heap.alloc(HeapObj::Str(result))?;
    vm.push(val); Ok(())
}
fn method_str_rstrip(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    let s = vm.recv_str(recv)?;
    let result = if pos.is_empty() { s.trim_end().to_string() }
        else { let p = vm.val_to_str(pos[0])?; s.trim_end_matches(|c| p.contains(c)).to_string() };
    let val = vm.heap.alloc(HeapObj::Str(result))?;
    vm.push(val); Ok(())
}
fn method_str_isdigit(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "isdigit");
    let s = vm.recv_str(recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))); Ok(())
}
fn method_str_isalpha(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "isalpha");
    let s = vm.recv_str(recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic()))); Ok(())
}
fn method_str_isalnum(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "isalnum");
    let s = vm.recv_str(recv)?;
    vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric()))); Ok(())
}
fn method_str_title(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    expect_args!(pos, 0, "title");
    let s = vm.recv_str(recv)?;
    let result = s.split_whitespace()
        .map(|w| { let mut cs = w.chars(); cs.next().map(|c| c.to_uppercase().to_string() + cs.as_str()).unwrap_or_default() })
        .collect::<Vec<_>>().join(" ");
    let val = vm.heap.alloc(HeapObj::Str(result))?;
    vm.push(val); Ok(())
}
fn method_str_center(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.is_empty() { return Err(cold_type("center() requires at least 1 argument(s)")); }
    let s = vm.recv_str(recv)?;
    if !pos[0].is_int() { return Err(cold_type("center() width must be an integer")); }
    let width = pos[0].as_int() as usize;
    let fill = if pos.len() > 1 { vm.val_to_str(pos[1])?.chars().next().unwrap_or(' ') } else { ' ' };
    let pad = width.saturating_sub(s.len());
    let left = pad / 2;
    let right = pad - left;
    let result = fill.to_string().repeat(left) + &s + &fill.to_string().repeat(right);
    let val = vm.heap.alloc(HeapObj::Str(result))?;
    vm.push(val); Ok(())
}
fn method_str_zfill(vm: &mut VM, recv: Val, pos: Vec<Val>, _kw: Vec<Val>) -> Result<(), VmErr> {
    if pos.is_empty() || !pos[0].is_int() { return Err(cold_type("zfill() requires an integer argument")); }
    let s = vm.recv_str(recv)?;
    let width = pos[0].as_int() as usize;
    let result = if s.len() >= width { s } else {
        let pad = "0".repeat(width - s.len());
        if s.starts_with('+') || s.starts_with('-') { s[..1].to_string() + &pad + &s[1..] }
        else { pad + &s }
    };
    let val = vm.heap.alloc(HeapObj::Str(result))?;
    vm.push(val); Ok(())
}
