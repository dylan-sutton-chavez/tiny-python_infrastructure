// vm/builtins.rs

use crate::s;

use super::VM;
use super::types::*;

use core::cell::RefCell;
use alloc::{string::{String, ToString}, vec::Vec, vec, rc::Rc};
use crate::modules::fx::FxHashSet as HashSet;

fn i64_to_radix(n: i64, radix: u32, prefix: &str) -> String {
    if n == 0 {
        let mut s = String::with_capacity(prefix.len() + 1);
        s.push_str(prefix); s.push('0'); return s;
    }
    let neg = n < 0;
    let mut abs = (n as i128).unsigned_abs();
    let mut buf = [0u8; 64];
    let mut i = buf.len();
    while abs > 0 {
        i -= 1;
        let d = (abs % radix as u128) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        abs /= radix as u128;
    }
    let mut s = String::with_capacity(prefix.len() + buf.len() - i + 1);
    if neg { s.push('-'); }
    s.push_str(prefix);
    s.push_str(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
    s
}

fn bigint_to_radix(b: &BigInt, radix: u32, prefix: &str) -> String {
    let mut out = String::new();
    if b.neg { out.push('-'); }
    out.push_str(prefix);
    if b.is_zero() { out.push('0'); return out; }

    let radix_big = BigInt::from_i64(radix as i64);
    let mut work = b.abs();
    let mut digits = Vec::<u8>::new();
    while !work.is_zero() {
        let (q, r) = work.divmod(&radix_big).unwrap();
        let d = r.to_i64_checked().unwrap() as u8;
        digits.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        work = q;
    }
    digits.reverse();
    out.push_str(unsafe { core::str::from_utf8_unchecked(&digits) });
    out
}

fn normalize_index(i: i64, len: usize) -> usize {
    (if i < 0 { len as i64 + i } else { i }) as usize
}

enum SliceSource { List(Vec<Val>), Tuple(Vec<Val>), Str(Vec<char>) }

impl SliceSource {
    fn len(&self) -> i64 {
        match self {
            Self::List(v)  => v.len() as i64,
            Self::Tuple(v) => v.len() as i64,
            Self::Str(v)   => v.len() as i64,
        }
    }
}

impl<'a> VM<'a> {

    #[inline]
    pub(super) fn mark_impure(&mut self) {
        if let Some(top) = self.observed_impure.last_mut() {
            *top = true;
        }
    }

    /* Pops N args, joins with space, appends to output buffer. */

    pub fn call_print(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        let mut out = String::new();
        for (i, v) in args.iter().enumerate() {
            if i > 0 { out.push(' '); }
            out.push_str(&self.display(*v));
        }
        self.output.push(out);
        Ok(())
    }

    /* Returns element count for strings, lists, tuples, dicts, sets, ranges. */

    pub fn call_len(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let n: i64 = if o.is_heap() { match self.heap.get(o) {
            HeapObj::Str(s) => s.chars().count() as i64,
            HeapObj::List(v) => v.borrow().len() as i64,
            HeapObj::Tuple(v) => v.len() as i64,
            HeapObj::Dict(v) => v.borrow().len() as i64,
            HeapObj::Set(v) => v.borrow().len() as i64,
            HeapObj::Range(s,e,st) => { let st=*st; ((e-s+st-st.signum())/st).max(0) }
            _ => return Err(cold_type("object has no len()")),
        }} else { return Err(cold_type("object has no len()")); };
        self.push(Val::int(n)); Ok(())
    }

    /* Returns absolute value for int and float operands. */
    
    pub fn call_abs(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if o.is_int() {
            let r = (o.as_int() as i128).abs();
            let v = self.i128_to_val(r)?;
            self.push(v);
        } else if o.is_float() {
            self.push(Val::float(o.as_float().abs()));
        } else if o.is_heap() {
            if let HeapObj::BigInt(b) = self.heap.get(o) {
                let ab = b.abs();
                let v = self.bigint_to_val(ab)?;
                self.push(v);
            } else {
                return Err(cold_type("abs() requires a number"));
            }
        } else {
            return Err(cold_type("abs() requires a number"));
        }
        Ok(())
    }

    /* Converts any value to its string representation via display. */
    
    pub fn call_str(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?; let s = self.display(o);
        let v = self.heap.alloc(HeapObj::Str(s))?; self.push(v); Ok(())
    }

    /* Converts float, bool, or parseable string to integer. */
    
    pub fn call_int(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if o.is_heap()
            && let HeapObj::BigInt(b) = self.heap.get(o) {
                let b = b.clone();
                let v = self.bigint_to_val(b)?;
                self.push(v);
                return Ok(());
        }
        let i = if o.is_int() { o.as_int() }
            else if o.is_float() { o.as_float() as i64 }
            else if o.is_bool() { o.as_bool() as i64 }
            else if o.is_heap() { match self.heap.get(o) {
                HeapObj::Str(s) => s.trim().parse().map_err(|_| cold_value("int(): invalid literal"))?,
                _ => return Err(cold_type("int() requires a number or string")),
            }}
            else { return Err(cold_type("int() requires a number or string")); };
        let v = self.bigint_to_val(BigInt::from_i64(i))?;
        self.push(v); Ok(())
    }

    /* Converts int or parseable string to floating point. */
    
    pub fn call_float(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let f = if o.is_float() { o.as_float() }
            else if o.is_bool() { o.as_bool() as i64 as f64 }
            else if o.is_int() { o.as_int() as f64 }
            else if o.is_heap() { match self.heap.get(o) {
                HeapObj::Str(s) => s.trim().parse().map_err(|_| cold_value("float(): invalid literal"))?,
                HeapObj::BigInt(b) => b.to_f64(),
                _ => return Err(cold_type("float() requires a number or string"))
            }}
            else { return Err(cold_type("float() requires a number or string")); };
        self.push(Val::float(f)); Ok(())
    }

    pub fn call_bool(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?; self.push(Val::bool(self.truthy(o))); Ok(())
    }

    pub fn call_type(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.type_name(o);
        let full = s!("<class '", str s, "'>");
        let v = self.heap.alloc(HeapObj::Str(full))?;
        self.push(v);
        Ok(())
    }

    pub fn call_chr(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if !o.is_int() { return Err(cold_type("chr() requires an integer")); }
        let c = char::from_u32(o.as_int() as u32).ok_or(cold_value("chr() arg out of range"))?;
        let mut s = String::with_capacity(4); // max UTF-8 char size
        s.push(c);
        let v = self.heap.alloc(HeapObj::Str(s))?; self.push(v); Ok(())
    }

    pub fn call_ord(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if o.is_heap()
            && let HeapObj::Str(s) = self.heap.get(o) {
                let mut cs = s.chars();
                if let (Some(c), None) = (cs.next(), cs.next()) {
                    self.push(Val::int(c as i64)); return Ok(());
                }
        }
        Err(cold_type("ord() requires string of length 1"))
    }

    /* Creates lazy Range(start, end, step) with 1-3 int arguments. */
    
    pub fn call_range(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        let gi = |v: Val| -> Result<i64, VmErr> {
            if v.is_int() { Ok(v.as_int()) } else { Err(cold_type("range() arguments must be integers")) }
        };
        let (s, e, st) = match args.len() {
            1 => (0, gi(args[0])?, 1),
            2 => (gi(args[0])?, gi(args[1])?, 1),
            3 => (gi(args[0])?, gi(args[1])?, gi(args[2])?),
            _ => return Err(cold_type("range() takes 1 to 3 arguments")),
        };
        if st == 0 { return Err(cold_value("range() step cannot be zero")); }
        let val = self.heap.alloc(HeapObj::Range(s, e, st))?;
        self.push(val); Ok(())
    }

    /* Rounds float to nearest int or to N decimal places. */
    
    pub fn call_round(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        let v = match (args.first(), args.get(1)) {
            (Some(o), Some(n)) if o.is_float() && n.is_int() => {
                let factor = fpowi(10.0, n.as_int() as i32);
                Val::float(fround(o.as_float() * factor) / factor)
            }
            (Some(o), None) if o.is_float() => Val::int(fround(o.as_float()) as i64),
            (Some(o), _) if o.is_int() => *o,
            (Some(o), _) if o.is_heap() && matches!(self.heap.get(*o), HeapObj::BigInt(_)) => *o,
            _ => return Err(cold_type("round() requires a number")),
        };
        self.push(v); Ok(())
    }

    /* Returns smallest or largest item from args or single iterable. */
    
    pub fn call_min(&mut self, op: u16) -> Result<(), VmErr> {
        let args: Vec<Val> = self.pop_n(op as usize)?;
        let items = self.unwrap_single_iterable(args)?;
        if items.is_empty() { return Err(cold_value("min() arg is an empty sequence")); }
        let m = items[1..].iter().try_fold(items[0], |m, &x| {
            self.lt_vals(x, m).map(|lt| if lt { x } else { m })
        })?;
        self.push(m); Ok(())
    }

    pub fn call_max(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        let items = self.unwrap_single_iterable(args)?;
        if items.is_empty() { return Err(cold_value("max() arg is an empty sequence")); }
        let m = items[1..].iter().try_fold(items[0], |m, &x| {
            self.lt_vals(m, x).map(|lt| if lt { x } else { m })
        })?;
        self.push(m); Ok(())
    }

    /* Sums iterable elements with optional start value. */
    
    pub fn call_sum(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        if args.is_empty() { return Err(cold_type("sum() requires at least 1 argument")); }
        let start = if args.len() > 1 { args[1] } else { Val::int(0) };
        let items = self.extract_iter(args[0], false)?;
        let mut acc = start;
        for item in items { acc = self.add_vals(acc, item)?; }
        self.push(acc); Ok(())
    }

    /* Returns new sorted list from iterable via comparison. */

    pub fn call_sorted(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let mut items = self.extract_iter(o, false)?;
        let mut sort_err: Option<VmErr> = None;
        items.sort_by(|&a, &b| {
            if sort_err.is_some() { return core::cmp::Ordering::Equal; }
            match self.lt_vals(a, b) {
                Ok(true) => core::cmp::Ordering::Less,
                Ok(false) => match self.lt_vals(b, a) {
                    Ok(true) => core::cmp::Ordering::Greater,
                    Ok(false) => core::cmp::Ordering::Equal,
                    Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
                },
                Err(e) => { sort_err = Some(e); core::cmp::Ordering::Equal }
            }
        });
        if let Some(e) = sort_err { return Err(e); }
        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
        self.push(val); Ok(())
    }

    /* Converts iterable to list or tuple, materializing lazy ranges. */

    pub fn call_list(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if o.is_heap()
            && let HeapObj::Str(s) = self.heap.get(o) {
                let s = s.clone();
                let items = self.str_to_char_vals(&s)?;
                let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
                self.push(val);
                return Ok(());
        }
        let items = self.extract_iter(o, true)?;
        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
        self.push(val); Ok(())
    }

    pub fn call_tuple(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let items: Vec<Val> = if o.is_heap() { match self.heap.get(o) {
            HeapObj::Tuple(v) => v.clone(),
            HeapObj::List(v)  => v.borrow().clone(),
            _ => return Err(cold_type("tuple() argument must be iterable")),
        }} else { return Err(cold_type("tuple() argument must be iterable")); };
        let val = self.heap.alloc(HeapObj::Tuple(items))?;
        self.push(val); Ok(())
    }

    /* Wraps iterable items as (index, value) tuple pairs. */

    pub fn call_enumerate(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let src = self.extract_iter(o, false)?;
        let mut pairs: Vec<Val> = Vec::with_capacity(src.len());
        for (i, x) in src.into_iter().enumerate() {
            let t = self.heap.alloc(HeapObj::Tuple(vec![Val::int(i as i64), x]))?;
            pairs.push(t);
        }
        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(pairs))))?;
        self.push(val); Ok(())
    }

    /* Pairs elements from N iterables into tuple list, truncating to shortest. */

    pub fn call_zip(&mut self, op: u16) -> Result<(), VmErr> {
        let mut iters: Vec<Vec<Val>> = Vec::with_capacity(op as usize);
        let mut vals = Vec::with_capacity(op as usize);
        for _ in 0..op { vals.push(self.pop()?); }
        vals.reverse();
        for v in vals { iters.push(self.extract_iter(v, false)?); }
        let len = iters.iter().map(|v| v.len()).min().unwrap_or(0);
        let mut pairs: Vec<Val> = Vec::with_capacity(len);
        for i in 0..len {
            let tuple: Vec<Val> = iters.iter().map(|v| v[i]).collect();
            let t = self.heap.alloc(HeapObj::Tuple(tuple))?;
            pairs.push(t);
        }
        let val = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(pairs))))?;
        self.push(val); Ok(())
    }

    /* Compares type_name string for sandbox-level type checking. */

    pub fn call_isinstance(&mut self) -> Result<(), VmErr> {
        let (arg2, obj) = (self.pop()?, self.pop()?);
        let obj_ty = self.type_name(obj);

        let obj_as_str: Option<String> = if obj.is_heap() {
            match self.heap.get(obj) {
                HeapObj::Str(s) => Some(s.clone()),
                _ => None,
            }
        } else { None };

        let check_one = |t: Val, heap: &HeapPool| -> Result<bool, VmErr> {
            if !t.is_heap() {
                return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types"));
            }
            match heap.get(t) {
                HeapObj::Type(name) => Ok(
                    name == obj_ty
                    || (obj_ty == "bool" && name == "int")
                    || obj_as_str.as_deref() == Some(name.as_str())
                ),
                HeapObj::NativeFn(id) => {
                    let name = id.name();
                    if !matches!(name, "int"|"str"|"float"|"bool"|"list"|"tuple"|"dict"|"set") {
                        return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types"));
                    }
                    Ok(
                        name == obj_ty
                        || (obj_ty == "bool" && name == "int")
                        || obj_as_str.as_deref() == Some(name)
                    )
                }
                _ => Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types")),
            }
        };

        if !arg2.is_heap() {
            return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types"));
        }

        let result = match self.heap.get(arg2) {
            HeapObj::Type(_) | HeapObj::NativeFn(_) => check_one(arg2, &self.heap)?,
            HeapObj::Tuple(items) => {
                let items: Vec<Val> = items.clone();
                items.iter().any(|&t| check_one(t, &self.heap).unwrap_or(false))
            }
            _ => return Err(VmErr::Type("isinstance() arg 2 must be a type or tuple of types")),
        };

        self.push(Val::bool(result));
        Ok(())
    }

    /* Returns empty string in sandbox; no stdin access in WASM. */

    pub fn call_input(&mut self) -> Result<(), VmErr> {
        let val = self.heap.alloc(HeapObj::Str(String::new()))?;
        self.push(val); Ok(())
    }

    // Shared helpers

    /* If single-arg is list/tuple/set, returns its items; otherwise returns args as-is. */
    
    fn unwrap_single_iterable(&self, args: Vec<Val>) -> Result<Vec<Val>, VmErr> {
        if args.len() == 1 && args[0].is_heap() {
            match self.heap.get(args[0]) {
                HeapObj::List(v) => return Ok(v.borrow().clone()),
                HeapObj::Tuple(v) => return Ok(v.clone()),
                HeapObj::Set(v) => return Ok(v.borrow().iter().cloned().collect()),
                _ => {}
            }
        }
        Ok(args)
    }

    /// Extracts Vec<Val> from list/tuple/set; optionally materializes Range.
    /// Str is handled at callsite (it needs heap-allocated chars, not ints).
    fn extract_iter(&self, o: Val, include_range: bool) -> Result<Vec<Val>, VmErr> {
        if !o.is_heap() { return Err(cold_type("object is not iterable")); }
        Ok(match self.heap.get(o) {
            HeapObj::List(v)  => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            HeapObj::Set(v)   => v.borrow().iter().cloned().collect(),
            HeapObj::Range(s, e, st) if include_range => {
                let (mut cur, end, step) = (*s, *e, *st);
                let mut out = Vec::new();
                if step > 0 { while cur < end { out.push(Val::int(cur)); cur += step; } }
                else        { while cur > end { out.push(Val::int(cur)); cur += step; } }
                out
            }
            _ => return Err(cold_type("object is not iterable")),
        })
    }

    fn alloc_set(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        let mut set = HashSet::with_capacity_and_hasher(items.len(), Default::default());
        for v in items { set.insert(v); }
        self.heap.alloc(HeapObj::Set(Rc::new(RefCell::new(set))))
    }

    pub fn build_set(&mut self, op: u16) -> Result<(), VmErr> {
        let items = self.pop_n(op as usize)?;
        let val = self.alloc_set(items)?;
        self.push(val); Ok(())
    }

    pub fn build_slice(&mut self, op: u16) -> Result<(), VmErr> {
        let step = if op == 3 { self.pop()? } else { Val::none() };
        let stop = self.pop()?;
        let start = self.pop()?;
        let val = self.heap.alloc(HeapObj::Slice(start, stop, step))?;
        self.push(val); Ok(())
    }

    pub fn unpack_ex(&mut self, op: u16) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(cold_type("cannot unpack non-iterable")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v) => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            _ => return Err(cold_type("cannot unpack non-iterable")),
        };
        let before = (op >> 8) as usize;
        let after = (op & 0xFF) as usize;
        if items.len() < before + after {
            return Err(cold_value("not enough values to unpack"));
        }
        let mid = items.len() - after;
        for &v in items[mid..].iter().rev() { self.push(v); }
        let star = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(
            items[before..mid].to_vec()
        ))))?;
        self.push(star);
        for &v in items[..before].iter().rev() { self.push(v); }
        Ok(())
    }

    pub fn call_dict(&mut self, op: u16) -> Result<(), VmErr> {
        if op == 0 {
            let val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(DictMap::new()))))?;
            self.push(val);
        } else {
            let args = self.pop_n((op as usize) * 2)?;
            let mut dm = DictMap::with_capacity(op as usize);
            for pair in args.chunks(2) { dm.insert(pair[0], pair[1]); }
            let val = self.heap.alloc(HeapObj::Dict(Rc::new(RefCell::new(dm))))?;
            self.push(val);
        }
        Ok(())
    }

    pub fn call_set(&mut self, op: u16) -> Result<(), VmErr> {
        if op == 0 {
            let val = self.alloc_set(Vec::new())?;
            self.push(val);
        } else {
            let o = self.pop()?;
            let src: Vec<Val> = if o.is_heap() {
                match self.heap.get(o) {
                    HeapObj::List(v)  => v.borrow().clone(),
                    HeapObj::Tuple(v) => v.clone(),
                    HeapObj::Set(v) => v.borrow().iter().cloned().collect(),
                    HeapObj::Str(s) => {
                        let s = s.clone();
                        self.str_to_char_vals(&s)?
                    },
                    _ => return Err(cold_type("set() argument must be iterable")),
                }
            } else {
                return Err(cold_type("set() argument must be iterable"));
            };
            let val = self.alloc_set(src)?;
            self.push(val);
        }
        Ok(())
    }

    pub fn get_item(&mut self) -> Result<bool, VmErr> {
        let idx = self.pop()?;
        let obj = self.pop()?;

        if idx.is_heap()
            && let HeapObj::Slice(start, stop, step) = self.heap.get(idx).clone() {
                let v = self.slice_val(obj, start, stop, step)?;
                self.push(v);
                return Ok(true);
        }

        if obj.is_heap() && idx.is_int()
            && let HeapObj::Str(s) = self.heap.get(obj) {
                let chars: Vec<char> = s.chars().collect();
                let i  = idx.as_int();
                let ui = normalize_index(i, chars.len());
                let c  = chars.get(ui).copied().ok_or(cold_value("string index out of range"))?;
                let val = self.heap.alloc(HeapObj::Str(c.to_string()))?;
                self.push(val);
                return Ok(true);
        }

        let v = self.getitem_val(obj, idx)?;
        self.push(v);
        Ok(false)
    }

    fn slice_val(&mut self, obj: Val, start: Val, stop: Val, step: Val) -> Result<Val, VmErr> {
        if !obj.is_heap() { return Err(cold_type("slice requires a sequence")); }
        let st = if step.is_none() { 1 } else if step.is_int() { step.as_int() } else {
            return Err(cold_type("slice step must be an integer"));
        };
        if st == 0 { return Err(cold_value("slice step cannot be zero")); }

        let source = match self.heap.get(obj) {
            HeapObj::List(v) => SliceSource::List(v.borrow().clone()),
            HeapObj::Tuple(v) => SliceSource::Tuple(v.clone()),
            HeapObj::Str(s) => SliceSource::Str(s.chars().collect()),
            _ => return Err(cold_type("object is not sliceable")),
        };

        let len = source.len();

        let clamp = |v: Val, def: i64| -> i64 {
            if v.is_none() { def }
            else if v.is_int() { let i = v.as_int(); if i < 0 { (len+i).max(0) } else { i.min(len) } }
            else { def }
        };

        let (s, e) = if st > 0 {
            (clamp(start, 0), clamp(stop, len))
        } else {
            (clamp(start, len - 1), clamp(stop, -1))
        };

        let mut indices = Vec::new();
        let mut cur = s;
        if st > 0 { while cur < e { indices.push(cur as usize); cur += st; } }
        else { while cur > e { indices.push(cur as usize); cur += st; } }

        let pick = |v: &[Val]| -> Vec<Val> {
            indices.iter().filter_map(|&i| v.get(i).copied()).collect()
        };

        match source {
            SliceSource::List(v)  => self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(pick(&v))))),
            SliceSource::Tuple(v) => self.heap.alloc(HeapObj::Tuple(pick(&v))),
            SliceSource::Str(chars) => {
                let sliced: String = indices.iter().filter_map(|&i| chars.get(i)).collect();
                self.heap.alloc(HeapObj::Str(sliced))
            }
        }
    }

    pub fn getitem_val(&self, obj: Val, idx: Val) -> Result<Val, VmErr> {
        if !obj.is_heap() { return Err(cold_type("object is not subscriptable")); }
        match self.heap.get(obj) {
            HeapObj::List(v) => {
                if !idx.is_int() { return Err(cold_type("list indices must be integers")); }
                let b = v.borrow(); let i = idx.as_int();
                let ui = normalize_index(i, b.len());
                b.get(ui).copied().ok_or(cold_value("list index out of range"))
            }
            HeapObj::Tuple(v) => {
                if !idx.is_int() { return Err(cold_type("tuple indices must be integers")); }
                let i = idx.as_int();
                let ui = normalize_index(i, v.len());
                v.get(ui).copied().ok_or(cold_value("tuple index out of range"))
            }
            HeapObj::Dict(p) => {
                p.borrow().get(&idx).copied()
                    .ok_or(cold_value("key not found"))
            }
            _ => Err(cold_type("object is not subscriptable")),
        }
    }

    pub fn store_item(&mut self) -> Result<(), VmErr> {
        let value = self.pop()?;
        let idx_val = self.pop()?;
        let cont = self.pop()?;
        if !cont.is_heap() { return Err(cold_type("object does not support item assignment")); }
        match self.heap.get_mut(cont) {
            HeapObj::List(v) => {
                if !idx_val.is_int() { return Err(cold_type("list indices must be integers")); }
                let mut b = v.borrow_mut();
                let i = idx_val.as_int();
                let ui = normalize_index(i, b.len());
                if ui >= b.len() { return Err(cold_value("list assignment index out of range")); }
                b[ui] = value;
            }
            HeapObj::Dict(p) => { p.borrow_mut().insert(idx_val, value); }
            HeapObj::Tuple(_) => return Err(cold_type("tuple does not support item assignment")),
            _ => return Err(cold_type("object does not support item assignment")),
        }
        Ok(())
    }

    // ─── Logical reductions ──────────────────────────────────────────

    pub fn call_all(&mut self, op: u16) -> Result<(), VmErr> {
        if op != 1 { return Err(cold_type("all() takes exactly 1 argument")); }
        let o = self.pop()?;
        let items = self.extract_iter(o, true)?;
        for v in items {
            if !self.truthy(v) {
                self.push(Val::bool(false));
                return Ok(());
            }
        }
        self.push(Val::bool(true));
        Ok(())
    }

    pub fn call_any(&mut self, op: u16) -> Result<(), VmErr> {
        if op != 1 { return Err(cold_type("any() takes exactly 1 argument")); }
        let o = self.pop()?;
        let items = self.extract_iter(o, true)?;
        for v in items {
            if self.truthy(v) {
                self.push(Val::bool(true));
                return Ok(());
            }
        }
        self.push(Val::bool(false));
        Ok(())
    }

    // ─── Number formatting ───────────────────────────────────────────

    pub fn call_bin(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.int_to_radix_string(o, 2, "0b")?;
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }
    pub fn call_oct(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.int_to_radix_string(o, 8, "0o")?;
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }
    pub fn call_hex(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.int_to_radix_string(o, 16, "0x")?;
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }

    /// Converts int/BigInt to "<prefix><digits>" with optional sign.
    fn int_to_radix_string(&self, v: Val, radix: u32, prefix: &str) -> Result<String, VmErr> {
        if v.is_int() {
            return Ok(i64_to_radix(v.as_int(), radix, prefix));
        }
        if v.is_heap()
            && let HeapObj::BigInt(b) = self.heap.get(v) {
                return Ok(bigint_to_radix(b, radix, prefix));
        }
        Err(cold_type("integer required"))
    }

    // ─── Arithmetic ──────────────────────────────────────────────────

    pub fn call_divmod(&mut self) -> Result<(), VmErr> {
        let b = self.pop()?;
        let a = self.pop()?;
        let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b))
            else { return Err(cold_type("divmod() requires integer operands")); };
        let (q, r) = ba.divmod(&bb).ok_or(VmErr::ZeroDiv)?;
        let qv = self.bigint_to_val(q)?;
        let rv = self.bigint_to_val(r)?;
        let val = self.heap.alloc(HeapObj::Tuple(vec![qv, rv]))?;
        self.push(val); Ok(())
    }

    pub fn call_pow(&mut self, op: u16) -> Result<(), VmErr> {
        let args = self.pop_n(op as usize)?;
        match args.len() {
            2 => {
                let r = self.pow_2arg(args[0], args[1])?;
                self.push(r);
                Ok(())
            }
            3 => {
                // Modular exponentiation: (a ** b) % c
                let (Some(base), Some(modulus)) =
                    (self.to_bigint(args[0]), self.to_bigint(args[2]))
                    else { return Err(cold_type("pow() with 3 args requires integers")); };
                if !args[1].is_int() {
                    return Err(cold_type("pow() with 3 args requires integer exponent"));
                }
                let mut e = args[1].as_int();
                if e < 0 { return Err(cold_value("pow() exponent must be non-negative")); }
                if modulus.is_zero() { return Err(VmErr::ZeroDiv); }

                let mut result = BigInt::from_i64(1);
                let (_, mut base) = base.divmod(&modulus).unwrap();
                while e > 0 {
                    if e & 1 == 1 {
                        let (_, r) = result.mul(&base).divmod(&modulus).unwrap();
                        result = r;
                    }
                    let (_, b2) = base.mul(&base).divmod(&modulus).unwrap();
                    base = b2;
                    e >>= 1;
                }
                let r = self.bigint_to_val(result)?;
                self.push(r);
                Ok(())
            }
            _ => Err(cold_type("pow() takes 2 or 3 arguments")),
        }
    }

    fn pow_2arg(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if let Some(ba) = self.to_bigint(a) && b.is_int() {
            let exp = b.as_int();
            if exp >= 0 {
                if exp > u32::MAX as i64 {
                    return Err(cold_value("pow() exponent too large"));
                }
                return self.bigint_to_val(ba.pow_u32(exp as u32));
            }
            return Ok(Val::float(fpowi(ba.to_f64(), exp as i32)));
        }
        let to_f = |v: Val| -> Result<f64, VmErr> {
            if v.is_int() { Ok(v.as_int() as f64) }
            else if v.is_float() { Ok(v.as_float()) }
            else { Err(cold_type("pow() requires numeric operands")) }
        };
        Ok(Val::float(fpowf(to_f(a)?, to_f(b)?)))
    }

    // ─── Introspection ───────────────────────────────────────────────

    pub fn call_repr(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let s = self.repr(o);
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }

    pub fn call_callable(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let result = if o.is_heap() {
            matches!(self.heap.get(o),
                HeapObj::Func(..) | HeapObj::BoundMethod(..) 
                | HeapObj::Type(_) | HeapObj::NativeFn(_))
        } else { false };
        self.push(Val::bool(result));
        Ok(())
    }

    pub fn call_id(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        // Use the NaN-boxed bit pattern as identity. Truncate to fit INT_MAX.
        let id = ((o.0 as i64).abs()) & Val::INT_MAX;
        self.push(Val::int(id));
        Ok(())
    }

    pub fn call_hash(&mut self) -> Result<(), VmErr> {
        use core::hash::{Hash, Hasher};
        let o = self.pop()?;
        let mut h = crate::modules::fx::FxHasher::default();
        if o.is_int()        { o.as_int().hash(&mut h); }
        else if o.is_float() { o.as_float().to_bits().hash(&mut h); }
        else if o.is_bool()  { o.as_bool().hash(&mut h); }
        else if o.is_none()  { 0u64.hash(&mut h); }
        else if o.is_heap() {
            match self.heap.get(o) {
                HeapObj::Str(s) => s.hash(&mut h),
                HeapObj::BigInt(b) => { b.neg.hash(&mut h); b.limbs.hash(&mut h); }
                HeapObj::Tuple(items) => {
                    for v in items { v.0.hash(&mut h); }
                }
                HeapObj::List(_) | HeapObj::Dict(_) | HeapObj::Set(_) => {
                    return Err(cold_type("unhashable type"));
                }
                _ => o.0.hash(&mut h),
            }
        }
        self.push(Val::int(h.finish() as i64 & Val::INT_MAX));
        Ok(())
    }

    // ─── Sequence ops ────────────────────────────────────────────────

    pub fn call_reversed(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        if !o.is_heap() { return Err(cold_type("reversed() requires a sequence")); }
        let mut items = if let HeapObj::Str(s) = self.heap.get(o) {
            let s = s.clone();
            self.str_to_char_vals(&s)?
        } else {
            self.extract_iter(o, true)?
        };
        items.reverse();
        let v = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(items))))?;
        self.push(v); Ok(())
    }

    // ─── format(value [, spec]) ─────────────────────────────────────

    pub fn call_format(&mut self, op: u16) -> Result<(), VmErr> {
        if op != 1 && op != 2 {
            return Err(cold_type("format() takes 1 or 2 arguments"));
        }
        let _spec = if op == 2 { Some(self.pop()?) } else { None };
        let val = self.pop()?;
        // Edge Python ignores spec details for now — uses display.
        // (Real format spec is just a small extension on top of this.)
        let s = self.display(val);
        let v = self.heap.alloc(HeapObj::Str(s))?;
        self.push(v); Ok(())
    }

    // ─── ascii(obj) — repr but with non-ASCII escaped ───────────────

    pub fn call_ascii(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        let r = self.repr(o);
        let mut out = String::with_capacity(r.len());
        for c in r.chars() {
            if (c as u32) < 0x80 {
                out.push(c);
            } else if (c as u32) < 0x10000 {
                out.push_str("\\u");
                let hex = i64_to_radix(c as i64, 16, "");
                for _ in 0..(4 - hex.len()) { out.push('0'); }
                out.push_str(&hex);
            } else {
                out.push_str("\\U");
                let hex = i64_to_radix(c as i64, 16, "");
                for _ in 0..(8 - hex.len()) { out.push('0'); }
                out.push_str(&hex);
            }
        }
        let v = self.heap.alloc(HeapObj::Str(out))?;
        self.push(v); Ok(())
    }

    // ─── getattr(obj, name [, default]) ─────────────────────────────

    pub fn call_getattr(&mut self, op: u16) -> Result<(), VmErr> {
        if op != 2 && op != 3 {
            return Err(cold_type("getattr() takes 2 or 3 arguments"));
        }
        let default = if op == 3 { Some(self.pop()?) } else { None };
        let name_val = self.pop()?;
        let obj = self.pop()?;

        let name = match (name_val.is_heap(), name_val.is_heap().then(|| self.heap.get(name_val))) {
            (true, Some(HeapObj::Str(s))) => s.clone(),
            _ => return Err(cold_type("getattr() name must be a string")),
        };

        let ty = self.type_name(obj);
        if let Some(method_id) = super::handlers::methods::lookup_method(ty, &name) {
            let bound = self.heap.alloc(HeapObj::BoundMethod(obj, method_id))?;
            self.push(bound);
            return Ok(());
        }
        if let Some(d) = default {
            self.push(d);
            return Ok(());
        }
        Err(cold_type("attribute not found"))
    }

    // ─── hasattr(obj, name) ─────────────────────────────────────────

    pub fn call_hasattr(&mut self) -> Result<(), VmErr> {
        let name_val = self.pop()?;
        let obj = self.pop()?;
        let name = match (name_val.is_heap(), name_val.is_heap().then(|| self.heap.get(name_val))) {
            (true, Some(HeapObj::Str(s))) => s.clone(),
            _ => return Err(cold_type("hasattr() name must be a string")),
        };
        let ty = self.type_name(obj);
        let exists = super::handlers::methods::lookup_method(ty, &name).is_some();
        self.push(Val::bool(exists));
        Ok(())
    }
}