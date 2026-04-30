// vm/handlers/methods.rs
//
// Single source of truth for built-in methods (str/list/dict).
// The macro generates the BuiltinMethodId enum, the name lookup, and the
// dispatch function from one declarative table. Adding a new method means
// adding one row here, nothing else.

use super::*;
use crate::alloc::string::ToString;

#[inline]
fn recv_str(vm: &VM, recv: Val) -> Result<String, VmErr> {
    match vm.heap.get(recv) {
        HeapObj::Str(s) => Ok(s.clone()),
        _ => Err(cold_type("method requires a string receiver")),
    }
}

#[inline]
fn val_to_str(vm: &VM, v: Val) -> Result<String, VmErr> {
    match vm.heap.get(v) {
        HeapObj::Str(s) => Ok(s.clone()),
        _ => Err(cold_type("argument must be a string")),
    }
}

#[inline]
fn check_arity(pos: &[Val], min: usize, max: usize, msg: &'static str) -> Result<(), VmErr> {
    if pos.len() < min || pos.len() > max {
        return Err(cold_type(msg));
    }
    Ok(())
}

#[inline]
fn list_clone(vm: &VM, recv: Val) -> Result<Vec<Val>, VmErr> {
    match vm.heap.get(recv) {
        HeapObj::List(rc) => Ok(rc.borrow().clone()),
        _ => Err(cold_type("method requires a list receiver")),
    }
}

#[inline]
fn dict_entries(vm: &VM, recv: Val) -> Result<Vec<(Val, Val)>, VmErr> {
    match vm.heap.get(recv) {
        HeapObj::Dict(rc) => Ok(rc.borrow().entries.clone()),
        _ => Err(cold_type("method requires a dict receiver")),
    }
}

#[inline]
fn capitalize_first(s: &str) -> String {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) => c.to_uppercase().to_string() + cs.as_str().to_lowercase().as_str(),
        None => String::new(),
    }
}

#[inline]
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            cs.next()
                .map(|c| c.to_uppercase().to_string() + cs.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl<'a> VM<'a> {
    pub(crate) fn handle_load_attr(&mut self, name_idx: u16, chunk: &SSAChunk) -> Result<(), VmErr> {
        let name = chunk.names.get(name_idx as usize)
            .ok_or(VmErr::Runtime("LoadAttr: bad name index"))?;
        let obj = self.pop()?;
        let ty = self.type_name(obj);
        let method_id = lookup_method(ty, name.as_str())
            .ok_or(VmErr::Type("'object' has no attribute"))?;
        let bound = self.heap.alloc(HeapObj::BoundMethod(obj, method_id))?;
        self.push(bound);
        Ok(())
    }
}

/// One macro that generates: enum, name lookup, dispatcher.
/// Each row: (Variant, "name", category, |vm, recv, pos| body)
/// Category `mutating` adds an automatic mark_impure() after the body.
macro_rules! define_methods {
    ( $( ($variant:ident, $name:literal, $cat:ident, |$vm:ident, $recv:ident, $pos:ident| $body:block) ),* $(,)? ) => {

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum BuiltinMethodId {
            $( $variant ),*
        }

        impl BuiltinMethodId {
            #[inline]
            pub fn name(self) -> &'static str {
                match self { $( Self::$variant => $name ),* }
            }
        }

        pub(crate) fn dispatch_method(
            vm: &mut VM, id: BuiltinMethodId,
            recv: Val, pos: Vec<Val>, kw: Vec<Val>,
        ) -> Result<(), VmErr> {
            if !kw.is_empty() {
                return Err(cold_type("builtin method takes no keyword arguments"));
            }
            match id {
                $(
                    BuiltinMethodId::$variant => {
                        let $vm = vm; let $recv = recv; let $pos = pos;
                        let result: Result<(), VmErr> = (|| $body)();
                        define_methods!(@maybe_impure $cat, $vm, result)
                    }
                ),*
            }
        }
    };

    (@maybe_impure mutating, $vm:ident, $r:ident) => {{
        if $r.is_ok() { $vm.mark_impure(); }
        $r
    }};
    (@maybe_impure pure, $vm:ident, $r:ident) => { $r };
}

/// (type, attr) -> BuiltinMethodId. Linear scan; only ~37 entries and not on
/// the hot path (CallMethod fusion bypasses LoadAttr+Call).
pub fn lookup_method(ty: &str, attr: &str) -> Option<BuiltinMethodId> {
    use BuiltinMethodId::*;
    Some(match (ty, attr) {
        ("dict", "get")        => DictGet,
        ("dict", "items")      => DictItems,
        ("dict", "keys")       => DictKeys,
        ("dict", "pop")        => DictPop,
        ("dict", "setdefault") => DictSetDefault,
        ("dict", "update")     => DictUpdate,
        ("dict", "values")     => DictValues,
        ("list", "append")     => ListAppend,
        ("list", "clear")      => ListClear,
        ("list", "copy")       => ListCopy,
        ("list", "count")      => ListCount,
        ("list", "extend")     => ListExtend,
        ("list", "index")      => ListIndex,
        ("list", "insert")     => ListInsert,
        ("list", "pop")        => ListPop,
        ("list", "remove")     => ListRemove,
        ("list", "reverse")    => ListReverse,
        ("list", "sort")       => ListSort,
        ("str", "capitalize")  => StrCapitalize,
        ("str", "center")      => StrCenter,
        ("str", "count")       => StrCount,
        ("str", "endswith")    => StrEndswith,
        ("str", "find")        => StrFind,
        ("str", "isalnum")     => StrIsAlnum,
        ("str", "isalpha")     => StrIsAlpha,
        ("str", "isdigit")     => StrIsDigit,
        ("str", "join")        => StrJoin,
        ("str", "lower")       => StrLower,
        ("str", "lstrip")      => StrLstrip,
        ("str", "replace")     => StrReplace,
        ("str", "rstrip")      => StrRstrip,
        ("str", "split")       => StrSplit,
        ("str", "startswith")  => StrStartswith,
        ("str", "strip")       => StrStrip,
        ("str", "title")       => StrTitle,
        ("str", "upper")       => StrUpper,
        ("str", "zfill")       => StrZfill,
        _ => return None,
    })
}

define_methods! {
    // ── str: zero-arg transforms ─────────────────────────────────────────
    (StrUpper, "upper", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "upper takes no arguments")?;
        let s = recv_str(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::Str(s.to_uppercase()))?;
        vm.push(v); Ok(())
    }),
    (StrLower, "lower", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "lower takes no arguments")?;
        let s = recv_str(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::Str(s.to_lowercase()))?;
        vm.push(v); Ok(())
    }),
    (StrStrip, "strip", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "strip takes no arguments")?;
        let s = recv_str(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::Str(s.trim().to_string()))?;
        vm.push(v); Ok(())
    }),
    (StrCapitalize, "capitalize", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "capitalize takes no arguments")?;
        let s = recv_str(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::Str(capitalize_first(&s)))?;
        vm.push(v); Ok(())
    }),
    (StrTitle, "title", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "title takes no arguments")?;
        let s = recv_str(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::Str(title_case(&s)))?;
        vm.push(v); Ok(())
    }),

    // ── str: optional separator ──────────────────────────────────────────
    (StrLstrip, "lstrip", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 1, "lstrip takes 0 or 1 arguments")?;
        let s = recv_str(vm, recv)?;
        let out = if pos.is_empty() {
            s.trim_start().to_string()
        } else {
            let p = val_to_str(vm, pos[0])?;
            s.trim_start_matches(|c| p.contains(c)).to_string()
        };
        let v = vm.heap.alloc(HeapObj::Str(out))?;
        vm.push(v); Ok(())
    }),
    (StrRstrip, "rstrip", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 1, "rstrip takes 0 or 1 arguments")?;
        let s = recv_str(vm, recv)?;
        let out = if pos.is_empty() {
            s.trim_end().to_string()
        } else {
            let p = val_to_str(vm, pos[0])?;
            s.trim_end_matches(|c| p.contains(c)).to_string()
        };
        let v = vm.heap.alloc(HeapObj::Str(out))?;
        vm.push(v); Ok(())
    }),

    // ── str: predicates ──────────────────────────────────────────────────
    (StrIsDigit, "isdigit", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "isdigit takes no arguments")?;
        let s = recv_str(vm, recv)?;
        vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())));
        Ok(())
    }),
    (StrIsAlpha, "isalpha", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "isalpha takes no arguments")?;
        let s = recv_str(vm, recv)?;
        vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())));
        Ok(())
    }),
    (StrIsAlnum, "isalnum", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "isalnum takes no arguments")?;
        let s = recv_str(vm, recv)?;
        vm.push(Val::bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())));
        Ok(())
    }),

    // ── str: queries with one string arg ─────────────────────────────────
    (StrStartswith, "startswith", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "startswith takes 1 argument")?;
        let s = recv_str(vm, recv)?;
        let p = val_to_str(vm, pos[0])?;
        vm.push(Val::bool(s.starts_with(p.as_str())));
        Ok(())
    }),
    (StrEndswith, "endswith", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "endswith takes 1 argument")?;
        let s = recv_str(vm, recv)?;
        let p = val_to_str(vm, pos[0])?;
        vm.push(Val::bool(s.ends_with(p.as_str())));
        Ok(())
    }),
    (StrFind, "find", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "find takes 1 argument")?;
        let s = recv_str(vm, recv)?;
        let sub = val_to_str(vm, pos[0])?;
        let idx = s.find(sub.as_str())
            .map(|i| s[..i].chars().count() as i64)
            .unwrap_or(-1);
        vm.push(Val::int(idx));
        Ok(())
    }),
    (StrCount, "count", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "count takes 1 argument")?;
        let s = recv_str(vm, recv)?;
        let sub = val_to_str(vm, pos[0])?;
        vm.push(Val::int(s.matches(sub.as_str()).count() as i64));
        Ok(())
    }),

    // ── str: split / join / replace ──────────────────────────────────────
    (StrSplit, "split", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 1, "split takes 0 or 1 arguments")?;
        let s = recv_str(vm, recv)?;
        let parts: Vec<Val> = if pos.is_empty() {
            s.split_whitespace()
                .map(|p| vm.heap.alloc(HeapObj::Str(p.to_string())))
                .collect::<Result<_, _>>()?
        } else {
            let sep = val_to_str(vm, pos[0])?;
            s.split(sep.as_str())
                .map(|p| vm.heap.alloc(HeapObj::Str(p.to_string())))
                .collect::<Result<_, _>>()?
        };
        let v = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(parts))))?;
        vm.push(v); Ok(())
    }),
    (StrJoin, "join", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "join takes 1 argument")?;
        let sep = recv_str(vm, recv)?;
        let items = match vm.heap.get(pos[0]) {
            HeapObj::List(rc) => rc.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            _ => return Err(cold_type("join() argument must be iterable")),
        };
        let mut parts: Vec<String> = Vec::with_capacity(items.len());
        for v in items { parts.push(val_to_str(vm, v)?); }
        let v = vm.heap.alloc(HeapObj::Str(parts.join(sep.as_str())))?;
        vm.push(v); Ok(())
    }),
    (StrReplace, "replace", pure, |vm, recv, pos| {
        check_arity(&pos, 2, 2, "replace takes 2 arguments")?;
        let s = recv_str(vm, recv)?;
        let old = val_to_str(vm, pos[0])?;
        let new = val_to_str(vm, pos[1])?;
        let v = vm.heap.alloc(HeapObj::Str(s.replace(old.as_str(), new.as_str())))?;
        vm.push(v); Ok(())
    }),

    // ── str: padding ─────────────────────────────────────────────────────
    (StrCenter, "center", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 2, "center takes 1 or 2 arguments")?;
        let s = recv_str(vm, recv)?;
        if !pos[0].is_int() { return Err(cold_type("center() width must be an integer")); }
        let width = pos[0].as_int() as usize;
        let fill = if pos.len() > 1 {
            val_to_str(vm, pos[1])?.chars().next().unwrap_or(' ')
        } else { ' ' };
        let pad = width.saturating_sub(s.len());
        let left = pad / 2;
        let right = pad - left;
        let out = fill.to_string().repeat(left) + &s + &fill.to_string().repeat(right);
        let v = vm.heap.alloc(HeapObj::Str(out))?;
        vm.push(v); Ok(())
    }),
    (StrZfill, "zfill", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "zfill takes 1 argument")?;
        if !pos[0].is_int() { return Err(cold_type("zfill() requires an integer argument")); }
        let s = recv_str(vm, recv)?;
        let width = pos[0].as_int() as usize;
        let out = if s.len() >= width {
            s
        } else {
            let pad = "0".repeat(width - s.len());
            if s.starts_with('+') || s.starts_with('-') {
                s[..1].to_string() + &pad + &s[1..]
            } else {
                pad + &s
            }
        };
        let v = vm.heap.alloc(HeapObj::Str(out))?;
        vm.push(v); Ok(())
    }),

    // ── list: pure ───────────────────────────────────────────────────────
    (ListIndex, "index", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "index takes 1 argument")?;
        let items = list_clone(vm, recv)?;
        let idx = items.iter()
            .position(|&v| eq_vals_with_heap(v, pos[0], &vm.heap))
            .map(|i| i as i64)
            .ok_or(cold_value("value not found in list"))?;
        vm.push(Val::int(idx));
        Ok(())
    }),
    (ListCount, "count", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "count takes 1 argument")?;
        let items = list_clone(vm, recv)?;
        let n = items.iter().filter(|&&v| eq_vals_with_heap(v, pos[0], &vm.heap)).count() as i64;
        vm.push(Val::int(n));
        Ok(())
    }),
    (ListCopy, "copy", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "copy takes no arguments")?;
        let items = list_clone(vm, recv)?;
        let v = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
        vm.push(v); Ok(())
    }),

    // ── list: mutating ───────────────────────────────────────────────────
    (ListAppend, "append", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "append takes 1 argument")?;
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => rc.borrow_mut().push(pos[0]),
            _ => return Err(cold_type("append: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListClear, "clear", mutating, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "clear takes no arguments")?;
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => rc.borrow_mut().clear(),
            _ => return Err(cold_type("clear: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListReverse, "reverse", mutating, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "reverse takes no arguments")?;
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => rc.borrow_mut().reverse(),
            _ => return Err(cold_type("reverse: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListExtend, "extend", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "extend takes 1 argument")?;
        let items: Vec<Val> = if pos[0].is_heap() {
            match vm.heap.get(pos[0]) {
                HeapObj::List(rc) => rc.borrow().clone(),
                HeapObj::Tuple(v) => v.clone(),
                _ => return Err(cold_type("extend() argument must be iterable")),
            }
        } else {
            return Err(cold_type("extend() argument must be iterable"));
        };
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => rc.borrow_mut().extend_from_slice(&items),
            _ => return Err(cold_type("extend: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListInsert, "insert", mutating, |vm, recv, pos| {
        check_arity(&pos, 2, 2, "insert takes 2 arguments")?;
        if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => {
                let mut b = rc.borrow_mut();
                let i = pos[0].as_int();
                let ui = if i < 0 {
                    (b.len() as i64 + i).max(0) as usize
                } else {
                    (i as usize).min(b.len())
                };
                b.insert(ui, pos[1]);
            }
            _ => return Err(cold_type("insert: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListRemove, "remove", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "remove takes 1 argument")?;
        let items = list_clone(vm, recv)?;
        let idx = items.iter()
            .position(|&v| eq_vals_with_heap(v, pos[0], &vm.heap))
            .ok_or(cold_value("list.remove: value not found"))?;
        match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => { rc.borrow_mut().remove(idx); }
            _ => return Err(cold_type("remove: receiver is not a list")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (ListPop, "pop", mutating, |vm, recv, pos| {
        check_arity(&pos, 0, 1, "pop takes 0 or 1 arguments")?;
        let popped = match vm.heap.get_mut(recv) {
            HeapObj::List(rc) => {
                let mut b = rc.borrow_mut();
                if b.is_empty() { return Err(cold_value("pop from empty list")); }
                if pos.is_empty() {
                    b.pop().unwrap()
                } else {
                    if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
                    let i = pos[0].as_int();
                    let ui = if i < 0 { (b.len() as i64 + i) as usize } else { i as usize };
                    if ui >= b.len() { return Err(cold_value("pop index out of range")); }
                    b.remove(ui)
                }
            }
            _ => return Err(cold_type("pop: receiver is not a list")),
        };
        vm.push(popped); Ok(())
    }),
    (ListSort, "sort", mutating, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "sort takes no arguments")?;
        let mut sorted = list_clone(vm, recv)?;
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
        vm.push(Val::none()); Ok(())
    }),

    // ── dict ─────────────────────────────────────────────────────────────
    (DictKeys, "keys", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "keys takes no arguments")?;
        let entries = dict_entries(vm, recv)?;
        let keys: Vec<Val> = entries.into_iter().map(|(k, _)| k).collect();
        let v = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(keys))))?;
        vm.push(v); Ok(())
    }),
    (DictValues, "values", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "values takes no arguments")?;
        let entries = dict_entries(vm, recv)?;
        let vals: Vec<Val> = entries.into_iter().map(|(_, v)| v).collect();
        let v = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(vals))))?;
        vm.push(v); Ok(())
    }),
    (DictItems, "items", pure, |vm, recv, pos| {
        check_arity(&pos, 0, 0, "items takes no arguments")?;
        let entries = dict_entries(vm, recv)?;
        let mut items: Vec<Val> = Vec::with_capacity(entries.len());
        for (k, vv) in entries {
            let t = vm.heap.alloc(HeapObj::Tuple(vec![k, vv]))?;
            items.push(t);
        }
        let v = vm.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
        vm.push(v); Ok(())
    }),
    (DictGet, "get", pure, |vm, recv, pos| {
        check_arity(&pos, 1, 2, "get takes 1 or 2 arguments")?;
        let default = if pos.len() == 2 { pos[1] } else { Val::none() };
        let result = match vm.heap.get(recv) {
            HeapObj::Dict(rc) => rc.borrow().get(&pos[0]).copied().unwrap_or(default),
            _ => return Err(cold_type("get: receiver is not a dict")),
        };
        vm.push(result); Ok(())
    }),
    (DictUpdate, "update", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 1, "update takes 1 argument")?;
        let pairs = match vm.heap.get(pos[0]) {
            HeapObj::Dict(rc) => rc.borrow().entries.clone(),
            _ => return Err(cold_type("update() argument must be a dict")),
        };
        match vm.heap.get_mut(recv) {
            HeapObj::Dict(rc) => {
                let mut b = rc.borrow_mut();
                for (k, v) in pairs { b.insert(k, v); }
            }
            _ => return Err(cold_type("update: receiver is not a dict")),
        }
        vm.push(Val::none()); Ok(())
    }),
    (DictPop, "pop", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 2, "pop takes 1 or 2 arguments")?;
        let default = if pos.len() == 2 { Some(pos[1]) } else { None };
        let result = match vm.heap.get_mut(recv) {
            HeapObj::Dict(rc) => {
                let mut b = rc.borrow_mut();
                if let Some(val) = b.remove(&pos[0]) {
                    val
                } else {
                    match default {
                        Some(d) => d,
                        None => return Err(cold_value("key not found")),
                    }
                }
            }
            _ => return Err(cold_type("pop: receiver is not a dict")),
        };
        vm.push(result); Ok(())
    }),
    (DictSetDefault, "setdefault", mutating, |vm, recv, pos| {
        check_arity(&pos, 1, 2, "setdefault takes 1 or 2 arguments")?;
        let default = if pos.len() > 1 { pos[1] } else { Val::none() };
        let result = match vm.heap.get_mut(recv) {
            HeapObj::Dict(rc) => {
                let already = rc.borrow().get(&pos[0]).copied();
                if let Some(v) = already {
                    v
                } else {
                    rc.borrow_mut().insert(pos[0], default);
                    default
                }
            }
            _ => return Err(cold_type("setdefault: receiver is not a dict")),
        };
        vm.push(result); Ok(())
    }),
}