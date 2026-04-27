// vm/ops.rs

use crate::s;

use super::types::*;

use alloc::{string::String, vec::Vec, rc::Rc};
use core::cell::RefCell;

// Coerces numeric pair to f64; None if neither is float.
fn coerce_floats(a: Val, b: Val) -> Option<(f64, f64)> {
    if !a.is_float() && !b.is_float() { return None; }
    let af = if a.is_float() { a.as_float() }
             else if a.is_int() { a.as_int() as f64 }
             else { return None };
    let bf = if b.is_float() { b.as_float() }
             else if b.is_int() { b.as_int() as f64 }
             else { return None };
    Some((af, bf))
}

/*
Cache Binop Macro
    Records heap type tags and promotes stable binary ops to fast path.
*/

macro_rules! cached_binop {
    ($heap:expr, $rip:expr, $opcode:expr, $a:expr, $b:expr, $cache:expr) => {{
        let ta = $heap.val_tag($a);
        let tb = $heap.val_tag($b);
        $cache.record($rip, $opcode, ta, tb);
    }};
}
pub(crate) use cached_binop;

/*
VM Value Helpers
    All methods that need &self or &mut self access to HeapPool.
*/

use super::VM;

impl<'a> VM<'a> {
    pub fn truthy(&self, v: Val) -> bool {
        if v.is_none() || v.is_false() { return false; }
        if v.is_true() { return true; }
        if v.is_int() { return v.as_int() != 0; }
        if v.is_float() { return v.as_float() != 0.0; }
        match self.heap.get(v) {
            HeapObj::Str(s) => !s.is_empty(),
            HeapObj::BigInt(b) => !b.is_zero(),
            HeapObj::List(l) => !l.borrow().is_empty(),
            HeapObj::Tuple(t) => !t.is_empty(),
            HeapObj::Dict(d) => !d.borrow().is_empty(),
            HeapObj::Set(s) => !s.borrow().is_empty(),
            HeapObj::Range(s,e,st) => if *st > 0 { s < e } else { s > e },
            HeapObj::Type(_) => true,
            HeapObj::Func(_, _, _) => true,
            HeapObj::Slice(..) => true,
            HeapObj::BoundMethod(..) => true,
        }
    }

    pub fn bitwise_op(&mut self, a: Val, b: Val, op: impl Fn(i64, i64) -> i64) -> Result<Val, VmErr> {
        if a.is_int() && b.is_int() {
            return Ok(Val::int(op(a.as_int(), b.as_int())));
        }
        let ai = self.to_bigint(a)
            .and_then(|b| b.to_i64_checked())
            .ok_or(VmErr::Type("bitwise op requires integer operands"))?;
        let bi = self.to_bigint(b)
            .and_then(|b| b.to_i64_checked())
            .ok_or(VmErr::Type("bitwise op requires integer operands"))?;
        Ok(Val::int(op(ai, bi)))
    }

    pub fn type_name(&self, v: Val) -> &'static str {
        if v.is_bool() { "bool" }
        else if v.is_int() { "int" }
        else if v.is_float() { "float" }
        else if v.is_none() { "NoneType" }
        else { match self.heap.get(v) {
            HeapObj::Str(_) => "str",
            HeapObj::BigInt(_) => "int",
            HeapObj::List(_) => "list",
            HeapObj::Dict(_) => "dict",
            HeapObj::Set(_) => "set",
            HeapObj::Tuple(_) => "tuple",
            HeapObj::Func(_, _, _) => "function",
            HeapObj::Type(_) => "type",
            HeapObj::Range(..) => "range",
            HeapObj::Slice(..) => "slice",
            HeapObj::BoundMethod(..) => "builtin_function_or_method",
        }}
    }

    fn append_reprs<'b>(&self, out: &mut String, it: impl Iterator<Item = &'b Val>) {
        let mut first = true;
        for v in it { if !first { out.push_str(", "); } out.push_str(&self.repr(*v)); first = false; }
    }

    pub fn display(&self, v: Val) -> String {
        if v.is_int() {
            let mut b = itoa::Buffer::new(); return b.format(v.as_int()).into();
        }
        if v.is_float() {
            let f = v.as_float();
            if f == 0.0 && f.is_sign_negative() {
                return "-0.0".into();
            }
            const I64_UPPER: f64 = i64::MAX as f64;
            if f.is_finite() && f >= (i64::MIN as f64) && f < I64_UPPER && f == (f as i64) as f64 {
                let i = f as i64;
                let mut b = itoa::Buffer::new();
                if !(Val::INT_MIN..=Val::INT_MAX).contains(&i) { return b.format(i).into(); }
                let mut s = String::new(); s.push_str(b.format(i)); s.push_str(".0"); return s;
            }
            let mut b = ryu::Buffer::new(); return b.format(f).into();
        }
        if v.is_true() { return "True".into(); }
        if v.is_false() { return "False".into(); }
        if v.is_none() { return "None".into(); }
        match self.heap.get(v) {
            HeapObj::Str(s) => s.clone(),
            HeapObj::BigInt(b) => b.to_decimal(),
            HeapObj::Type(name)    => s!("<class '", str name, "'>"),
            HeapObj::Func(i,_,_)   => s!("<function ", int *i),
            HeapObj::Slice(s,e,st) => s!("slice(", str &self.display(*s), ", ", str &self.display(*e), ", ", str &self.display(*st), ")"),
            HeapObj::Range(s,e,st) => if *st == 1 { s!("range(", int *s, ", ", int *e, ")") } else { s!("range(", int *s, ", ", int *e, ", ", int *st, ")") },
            HeapObj::List(l) => { let mut o = s!(cap: 32; "["); self.append_reprs(&mut o, l.borrow().iter()); o.push(']'); o },
            HeapObj::Tuple(t) => if t.len() == 1 { s!("(", str &self.repr(t[0]), ",)") } else { let mut o = s!(cap: 32; "("); self.append_reprs(&mut o, t.iter()); o.push(')'); o },
            HeapObj::Dict(d) => { let mut o = s!(cap: 32; "{"); for (i,(k,v)) in d.borrow().iter().enumerate() { if i>0 { o.push_str(", "); } o.push_str(&self.repr(k)); o.push_str(": "); o.push_str(&self.repr(v)); } o.push('}'); o },
            HeapObj::BoundMethod(_, id) => s!("<built-in method ", str id.name(), ">"),
            HeapObj::Set(s) => {
                let mut items: Vec<Val> = s.borrow().iter().cloned().collect();
                if items.is_empty() { return "set()".into(); }
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
                let mut out = String::new();
                out.push('{');
                self.append_reprs(&mut out, items.iter());
                out.push('}');
                out
            }
        }
    }

    pub fn repr(&self, v: Val) -> String {
        if v.is_heap() && let HeapObj::Str(s) = self.heap.get(v) { return s!("'", str s, "'"); }
        self.display(v)
    }

    pub fn lt_vals(&self, a: Val, b: Val) -> Result<bool, VmErr> {
        let a = if a.is_bool() { Val::int(a.as_bool() as i64) } else { a };
        let b = if b.is_bool() { Val::int(b.as_bool() as i64) } else { b };
        if a.is_int() && b.is_int() { return Ok(a.as_int() < b.as_int()); }
        if let Some((af, bf)) = coerce_floats(a, b) { return Ok(af < bf); }
        if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
            return Ok(ba.cmp(&bb) == core::cmp::Ordering::Less);
        }
        if a.is_heap() && b.is_heap()
            && let (HeapObj::Str(x), HeapObj::Str(y)) = (self.heap.get(a), self.heap.get(b)) {
                return Ok(x < y);
        }
        Err(VmErr::Type("'<' not supported between these types"))
    }

    // Checks item presence in list, tuple, dict, set, or substring in string.
    pub fn contains(&self, container: Val, item: Val) -> bool {
        if !container.is_heap() { return false; }
        match self.heap.get(container) {
            HeapObj::List(v) => v.borrow().iter().any(|x| eq_vals_with_heap(*x, item, &self.heap)),
            HeapObj::Tuple(v) => v.iter().any(|x| eq_vals_with_heap(*x, item, &self.heap)),
            HeapObj::Dict(p) => p.borrow().contains_key(&item),
            HeapObj::Set(s) => s.borrow().iter().any(|x| eq_vals_with_heap(*x, item, &self.heap)),
            HeapObj::Str(s) => {
                if item.is_heap() && let HeapObj::Str(sub) = self.heap.get(item) { return s.contains(sub.as_str()); }
                false
            }
            _ => false
        }
    }
    pub fn add_vals(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if a.is_int() && b.is_int() {
            return match a.as_int().checked_add(b.as_int()) {
                Some(r) if (Val::INT_MIN..=Val::INT_MAX).contains(&r) => Ok(Val::int(r)),
                Some(r) => self.heap.alloc(HeapObj::BigInt(BigInt::from_i64(r))),
                None    => self.i128_to_val(a.as_int() as i128 + b.as_int() as i128),
            };
        }
        if let Some((af, bf)) = coerce_floats(a, b) { return Ok(Val::float(af + bf)); }
        if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
            return self.bigint_to_val(ba.add(&bb));
        }
        if a.is_heap() && b.is_heap() {
            match (self.heap.get(a), self.heap.get(b)) {
                (HeapObj::Str(sa), HeapObj::Str(sb)) => {
                    let mut r = String::with_capacity(sa.len() + sb.len());
                    r.push_str(sa);
                    r.push_str(sb);
                    return self.heap.alloc(HeapObj::Str(r));
                }
                (HeapObj::List(va), HeapObj::List(vb)) => {
                    let mut lst = va.borrow().clone(); lst.extend_from_slice(&vb.borrow());
                    return self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(lst))));
                }
                (HeapObj::Tuple(va), HeapObj::Tuple(vb)) => {
                    let mut tup = va.clone(); tup.extend_from_slice(vb);
                    return self.heap.alloc(HeapObj::Tuple(tup));
                }
                _ => {}
            }
        }
        Err(VmErr::Type("unsupported operand type(s) for '+'"))
    }

    pub fn sub_vals(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if a.is_int() && b.is_int() {
            return match a.as_int().checked_sub(b.as_int()) {
                Some(r) if (Val::INT_MIN..=Val::INT_MAX).contains(&r) => Ok(Val::int(r)),
                Some(r) => self.heap.alloc(HeapObj::BigInt(BigInt::from_i64(r))),
                None    => self.i128_to_val(a.as_int() as i128 - b.as_int() as i128),
            };
        }
        if let Some((af, bf)) = coerce_floats(a, b) { return Ok(Val::float(af - bf)); }
        if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
            return self.bigint_to_val(ba.sub(&bb));
        }
        Err(VmErr::Type("unsupported operand type(s) for '-'"))
    }

    pub fn mul_vals(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if a.is_int() && b.is_int() {
            return match a.as_int().checked_mul(b.as_int()) {
                Some(r) if (Val::INT_MIN..=Val::INT_MAX).contains(&r) => Ok(Val::int(r)),
                Some(r) => self.heap.alloc(HeapObj::BigInt(BigInt::from_i64(r))),
                None    => self.i128_to_val(a.as_int() as i128 * b.as_int() as i128),
            };
        }
        if let Some((af, bf)) = coerce_floats(a, b) { return Ok(Val::float(af * bf)); }
        if let (Some(ba), Some(bb)) = (self.to_bigint(a), self.to_bigint(b)) {
            return self.bigint_to_val(ba.mul(&bb));
        }
        let (seq_val, count) = if a.is_heap() && b.is_int() { (a, b.as_int()) }
                else if a.is_int() && b.is_heap() { (b, a.as_int()) }
                else { return Err(VmErr::Type("unsupported operand type(s) for '*'")); };
        let n = count.max(0) as usize;
        match self.heap.get(seq_val) {
            HeapObj::Str(s) => {
                let r = s.repeat(n);
                return self.heap.alloc(HeapObj::Str(r));
            }
            HeapObj::List(rc) => {
                let src = rc.borrow().clone();
                let mut out = Vec::with_capacity(src.len() * n);
                for _ in 0..n { out.extend_from_slice(&src); }
                return self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(out))));
            }
            HeapObj::Tuple(v) => {
                let src = v.clone();
                let mut out = Vec::with_capacity(src.len() * n);
                for _ in 0..n { out.extend_from_slice(&src); }
                return self.heap.alloc(HeapObj::Tuple(out));
            }
            _ => {}
        }
        Err(VmErr::Type("unsupported operand type(s) for '*'"))
    }

    pub fn div_vals(&self, a: Val, b: Val) -> Result<Val, VmErr> {
        let bv = self.to_f64_coerce(b).map_err(|_| VmErr::Type("'/' requires numeric operands"))?;
        if bv == 0.0 { return Err(VmErr::ZeroDiv); }
        let av = self.to_f64_coerce(a).map_err(|_| VmErr::Type("'/' requires numeric operands"))?;
        Ok(Val::float(av / bv))
    }

    pub(crate) fn to_bigint(&self, v: Val) -> Option<BigInt> {
        if v.is_int() { return Some(BigInt::from_i64(v.as_int())); }
        if v.is_heap()
            && let HeapObj::BigInt(b) = self.heap.get(v) { return Some(b.clone()); }
        None
    }

    pub(crate) fn bigint_to_val(&mut self, b: BigInt) -> Result<Val, VmErr> {
        if let Some(i) = b.to_i64_checked()
            && (Val::INT_MIN..=Val::INT_MAX).contains(&i) { return Ok(Val::int(i)); }
        self.heap.alloc(HeapObj::BigInt(b))
    }

    fn to_f64_coerce(&self, v: Val) -> Result<f64, VmErr> {
        if v.is_int() { return Ok(v.as_int() as f64); }
        if v.is_float() { return Ok(v.as_float()); }
        if v.is_heap()
            && let HeapObj::BigInt(b) = self.heap.get(v) { return Ok(b.to_f64()); }
        Err(VmErr::Type("numeric operand required"))
    }

    pub(crate) fn i128_to_val(&mut self, r: i128) -> Result<Val, VmErr> {
        if r >= Val::INT_MIN as i128 && r <= Val::INT_MAX as i128 {
            return Ok(Val::int(r as i64));
        }
        self.heap.alloc(HeapObj::BigInt(BigInt::from_i128(r)))
    }
}