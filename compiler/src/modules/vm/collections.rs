// vm/collections.rs

use super::VM;
use super::types::*;
use alloc::{string::ToString, vec::Vec, rc::Rc, string::String};
use hashbrown::HashSet;
use core::cell::RefCell;

// Resolves negative index relative to sequence length.
#[inline]
fn normalize_index(i: i64, len: usize) -> usize {
    (if i < 0 { len as i64 + i } else { i }) as usize
}

enum SliceSource { List(Vec<Val>), Tuple(Vec<Val>), Str(Vec<char>) } // Extract type and data once

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
    
    fn alloc_set(&mut self, items: Vec<Val>) -> Result<Val, VmErr> {
        let mut set = HashSet::with_capacity(items.len());
        for v in items { set.insert(v); }
        self.heap.alloc(HeapObj::Set(Rc::new(RefCell::new(set))))
    }

    /*
    BuildSet
        Pops N items, deduplicates preserving order, pushes HeapObj::Set.
    */
    
    pub fn build_set(&mut self, op: u16) -> Result<(), VmErr> {
        let items = self.pop_n(op as usize)?;
        let val = self.alloc_set(items)?;
        self.push(val); Ok(())
    }

    /*
    BuildSlice
        Pops 2 or 3 items (start, stop, [step]), pushes HeapObj::Slice.
    */

    pub fn build_slice(&mut self, op: u16) -> Result<(), VmErr> {
        let step = if op == 3 { self.pop()? } else { Val::none() };
        let stop = self.pop()?;
        let start = self.pop()?;
        let val = self.heap.alloc(HeapObj::Slice(start, stop, step))?;
        self.push(val); Ok(())
    }

    /*
    UnpackEx
        Extended unpacking with star target: a, *b, c = iterable.
        Operand encodes (before << 8) | after for positional counts.
    */

    pub fn unpack_ex(&mut self, op: u16) -> Result<(), VmErr> {
        let obj = self.pop()?;
        if !obj.is_heap() { return Err(VmErr::Type("cannot unpack non-iterable")); }
        let items: Vec<Val> = match self.heap.get(obj) {
            HeapObj::List(v) => v.borrow().clone(),
            HeapObj::Tuple(v) => v.clone(),
            _ => return Err(VmErr::Type("cannot unpack non-iterable")),
        };
        let before = (op >> 8) as usize;
        let after = (op & 0xFF) as usize;
        if items.len() < before + after {
            return Err(VmErr::Value("not enough values to unpack"));
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

    /*
    CallDict
        Constructs dict from keyword args or empty; operand = pair count.
    */

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

    /*
    CallSet
        Constructs set from iterable arg or empty set.
    */

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
                    HeapObj::Set(v)   => v.borrow().iter().cloned().collect(),
                    HeapObj::Str(s) => {
                        let s = s.clone();
                        self.str_to_char_vals(&s)?
                    },
                    _ => return Err(VmErr::Type("set() argument must be iterable")),
                }
            } else {
                return Err(VmErr::Type("set() argument must be iterable"));
            };
            let val = self.alloc_set(src)?;
            self.push(val);
        }
        Ok(())
    }

    /*
    GetItem Dispatch
        Handles Str[int], Slice subscript, and delegates to getitem_val.
    */

    pub fn get_item(&mut self) -> Result<bool, VmErr> {
        let idx = self.pop()?;
        let obj = self.pop()?;

        // Slice dispatch
        if idx.is_heap()
            && let HeapObj::Slice(start, stop, step) = self.heap.get(idx).clone() {
                let v = self.slice_val(obj, start, stop, step)?;
                self.push(v);
                return Ok(true);
        }

        // Str[int] needs heap alloc
        if obj.is_heap() && idx.is_int()
            && let HeapObj::Str(s) = self.heap.get(obj) {
                let chars: Vec<char> = s.chars().collect();
                let i  = idx.as_int();
                let ui = normalize_index(i, chars.len());
                let c  = chars.get(ui).copied().ok_or(VmErr::Value("string index out of range"))?;
                let val = self.heap.alloc(HeapObj::Str(c.to_string()))?;
                self.push(val);
                return Ok(true);
        }

        let v = self.getitem_val(obj, idx)?;
        self.push(v);
        Ok(false)
    }

    /*
    Slice Value
        Single heap pass via SliceSource, extracts sub-sequence preserving type.
    */

    fn slice_val(&mut self, obj: Val, start: Val, stop: Val, step: Val) -> Result<Val, VmErr> {
        if !obj.is_heap() { return Err(VmErr::Type("slice requires a sequence")); }
        let st = if step.is_none() { 1 } else if step.is_int() { step.as_int() } else {
            return Err(VmErr::Type("slice step must be an integer"));
        };
        if st == 0 { return Err(VmErr::Value("slice step cannot be zero")); }

        let source = match self.heap.get(obj) {
            HeapObj::List(v) => SliceSource::List(v.borrow().clone()),
            HeapObj::Tuple(v) => SliceSource::Tuple(v.clone()),
            HeapObj::Str(s) => SliceSource::Str(s.chars().collect()),
            _ => return Err(VmErr::Type("object is not sliceable")),
        };

        let len = source.len();

        let clamp = |v: Val, def: i64| -> i64 {
            if v.is_none() { def }
            else if v.is_int() { let i = v.as_int(); if i < 0 { (len+i).max(0) } else { i.min(len) } }
            else { def }
        };
        let (s, e) = if st > 0 { (clamp(start, 0), clamp(stop, len)) }
                    else { (clamp(start, len-1), clamp(stop, -1)) };

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

    /*
    GetItem Value
        Index dispatch for list[int], tuple[int], dict[key].
    */

    pub fn getitem_val(&self, obj: Val, idx: Val) -> Result<Val, VmErr> {
        if !obj.is_heap() { return Err(VmErr::Type("object is not subscriptable")); }
        match self.heap.get(obj) {
            HeapObj::List(v) => {
                if !idx.is_int() { return Err(VmErr::Type("list indices must be integers")); }
                let b = v.borrow(); let i = idx.as_int();
                let ui = normalize_index(i, b.len());
                b.get(ui).copied().ok_or(VmErr::Value("list index out of range"))
            }
            HeapObj::Tuple(v) => {
                if !idx.is_int() { return Err(VmErr::Type("tuple indices must be integers")); }
                let i = idx.as_int();
                let ui = normalize_index(i, v.len());
                v.get(ui).copied().ok_or(VmErr::Value("tuple index out of range"))
            }
            HeapObj::Dict(p) => {
                p.borrow().get(&idx).copied()
                    .ok_or(VmErr::Value("key not found"))
            }
            _ => Err(VmErr::Type("object is not subscriptable")),
        }
    }

    /*
    StoreItem
        Mutates list[int], dict[key], or rejects tuple assignment.
    */
    
    pub fn store_item(&mut self) -> Result<(), VmErr> {
        let value = self.pop()?;
        let idx_val = self.pop()?;
        let cont = self.pop()?;
        if !cont.is_heap() { return Err(VmErr::Type("object does not support item assignment")); }
        match self.heap.get_mut(cont) {
            HeapObj::List(v) => {
                if !idx_val.is_int() { return Err(VmErr::Type("list indices must be integers")); }
                let mut b = v.borrow_mut();
                let i = idx_val.as_int();
                let ui = normalize_index(i, b.len());
                if ui >= b.len() { return Err(VmErr::Value("list assignment index out of range")); }
                b[ui] = value;
            }
            HeapObj::Dict(p) => { p.borrow_mut().insert(idx_val, value); }
            HeapObj::Tuple(_) => return Err(VmErr::Type("tuple does not support item assignment")),
            _ => return Err(VmErr::Type("object does not support item assignment")),
        }
        Ok(())
    }
}