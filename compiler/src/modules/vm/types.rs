// vm/types.rs

use alloc::{string::{String}, vec::Vec, vec, rc::Rc};
use core::{fmt, cell::RefCell};
use crate::modules::fx::{FxHashMap as HashMap, FxHashSet as HashSet};

/*
Sandbox Limits
    Configurable call depth, operation budget and heap quota per execution.
*/

pub struct Limits { pub calls: usize, pub ops: usize, pub heap: usize }

impl Limits {
    pub fn none() -> Self { Self { calls: 1_000, ops: usize::MAX, heap: 10_000_000 } }
    pub fn sandbox() -> Self { Self { calls: 256, ops: 100_000_000, heap: 100_000 } }
}

/*
Val
    NaN-boxed 8-byte value: int, float, bool, None or heap index inline.
*/

const QNAN: u64 = 0x7FFC_0000_0000_0000;
const SIGN: u64 = 0x8000_0000_0000_0000;
const TAG_NONE: u64 = QNAN | 1;
const TAG_TRUE: u64 = QNAN | 2;
const TAG_FALSE: u64 = QNAN | 3;
const TAG_INT: u64 = QNAN | SIGN;
const TAG_HEAP: u64 = QNAN | 4;

#[derive(Clone, Copy, Debug)]
pub struct Val(pub(crate) u64);

impl PartialEq for Val {
    #[inline] fn eq(&self, o: &Self) -> bool { self.0 == o.0 }
}
impl Eq for Val {}

impl core::hash::Hash for Val {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Val {
    #[inline(always)] pub fn float(f: f64) -> Self {
        let bits = f.to_bits();
        if (bits & QNAN) == QNAN { Self(QNAN) } else { Self(bits) }
    }
    #[inline(always)]
    pub fn is_numeric(&self) -> bool {
        self.is_int() || self.is_float()
    }
    pub const INT_MAX: i64 =  0x0000_7FFF_FFFF_FFFF;
    pub const INT_MIN: i64 = -0x0000_8000_0000_0000;
    #[inline(always)] pub fn int(i: i64) -> Self {
        Self(TAG_INT | (i as u64 & 0x0000_FFFF_FFFF_FFFF))
    }
    #[inline(always)] pub fn int_checked(i: i64) -> Option<Self> {
        if !(Self::INT_MIN..=Self::INT_MAX).contains(&i) { None } else { Some(Self::int(i)) }
    }
    #[inline(always)] pub fn none() -> Self { Self(TAG_NONE) }
    #[inline(always)] pub fn bool(b: bool) -> Self { Self(if b { TAG_TRUE } else { TAG_FALSE }) }
    #[inline(always)] pub fn heap(idx: u32) -> Self { Self(TAG_HEAP | ((idx as u64) << 4)) }

    #[inline(always)] pub fn is_float(&self) -> bool { (self.0 & QNAN) != QNAN }
    #[inline(always)] pub fn is_int(&self) -> bool { (self.0 & (QNAN | SIGN)) == TAG_INT }
    #[inline(always)] pub fn is_none(&self) -> bool { self.0 == TAG_NONE }
    #[inline(always)] pub fn is_true(&self) -> bool { self.0 == TAG_TRUE }
    #[inline(always)] pub fn is_false(&self) -> bool { self.0 == TAG_FALSE }
    #[inline(always)] pub fn is_bool(&self) -> bool { self.0 == TAG_TRUE || self.0 == TAG_FALSE }
    #[inline(always)] pub fn is_heap(&self) -> bool {
        (self.0 & QNAN) == QNAN && (self.0 & SIGN) == 0 && (self.0 & 0xF) >= 4
    }

    #[inline(always)] pub fn as_float(&self) -> f64  { f64::from_bits(self.0) }
    #[inline(always)] pub fn as_int(&self) -> i64  {
        let raw = (self.0 & 0x0000_FFFF_FFFF_FFFF) as i64;
        (raw << 16) >> 16
    }
    #[inline(always)] pub fn as_bool(&self) -> bool { self.0 == TAG_TRUE }
    #[inline(always)] pub fn as_heap(&self) -> u32 { ((self.0 >> 4) & 0x0FFF_FFFF) as u32 }
}

/*
BigInt Arbitrary Precision Integer
    Implements signed arbitrary precision integers using base-2^32 little-endian limb storage.
*/

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    pub neg: bool,
    pub limbs: Vec<u32>,
}

impl BigInt {
    pub fn zero() -> Self { Self { neg: false, limbs: Vec::new() } }
    pub fn is_zero(&self) -> bool { self.limbs.is_empty() }

    pub fn shl_u32(&self, shift: u32) -> Self {
        if self.is_zero() || shift == 0 { return self.clone(); }
        let limb_shift = (shift / 32) as usize;
        let bit_shift  = shift % 32;
        let mut limbs = vec![0u32; limb_shift];
        let mut carry = 0u64;
        for &l in &self.limbs {
            let cur = (l as u64) << bit_shift | carry;
            limbs.push(cur as u32);
            carry = cur >> 32;
        }
        if carry != 0 { limbs.push(carry as u32); }
        Self { neg: self.neg, limbs }
    }

    pub fn shr_u32(&self, shift: u32) -> Self {
        if self.is_zero() || shift == 0 { return self.clone(); }
        let limb_shift = (shift / 32) as usize;
        let bit_shift  = shift % 32;
        if limb_shift >= self.limbs.len() { return Self::zero(); }
        let mut limbs: Vec<u32> = Vec::new();
        for i in limb_shift..self.limbs.len() {
            let mut word = self.limbs[i] >> bit_shift;
            if bit_shift > 0 && i + 1 < self.limbs.len() {
                word |= self.limbs[i + 1] << (32 - bit_shift);
            }
            limbs.push(word);
        }
        Self::trim(&mut limbs);
        Self { neg: self.neg, limbs }
    }

    pub fn from_i64(v: i64) -> Self {
        if v == 0 { return Self::zero(); }
        let neg = v < 0;
        let abs = (v as i128).unsigned_abs() as u64;
        let mut limbs = vec![(abs & 0xFFFF_FFFF) as u32];
        if abs >> 32 != 0 { limbs.push((abs >> 32) as u32); }
        Self { neg, limbs }
    }

    pub fn from_i128(v: i128) -> Self {
        if v == 0 { return Self::zero(); }
        let neg = v < 0;
        let mut abs = v.unsigned_abs();
        let mut limbs = Vec::new();
        while abs != 0 { limbs.push((abs & 0xFFFF_FFFF) as u32); abs >>= 32; }
        Self { neg, limbs }
    }

    pub fn from_decimal(s: &str) -> Self {
        let (neg, digits) = if let Some(stripped) = s.strip_prefix('-') { (true, stripped) } else { (false, s) };
        let mut r = Self::zero();
        for c in digits.chars() {
            let d = (c as u8).wrapping_sub(b'0') as u32;
            r = r.mul_u32(10);
            if d != 0 { r = r.add(&Self { neg: false, limbs: vec![d] }); }
        }
        r.neg = neg && !r.is_zero();
        r
    }

    pub fn to_i64_checked(&self) -> Option<i64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(if self.neg { -(self.limbs[0] as i64) } else { self.limbs[0] as i64 }),
            2 => {
                let abs = self.limbs[0] as u64 | ((self.limbs[1] as u64) << 32);
                if self.neg {
                    if abs > i64::MIN.unsigned_abs() { 
                        None 
                    } else if abs == i64::MIN.unsigned_abs() {
                        Some(i64::MIN)
                    } else { 
                        Some(-(abs as i64)) 
                    }
                } else {
                    if abs > i64::MAX as u64 { None } else { Some(abs as i64) }
                }
            }
            _ => None,
        }
    }

    pub fn to_f64(&self) -> f64 {
        let (mut r, mut base) = (0.0f64, 1.0f64);
        for &l in &self.limbs { r += l as f64 * base; base *= 4_294_967_296.0; }
        if self.neg { -r } else { r }
    }

    pub fn neg(&self) -> Self {
        if self.is_zero() { self.clone() }
        else { Self { neg: !self.neg, limbs: self.limbs.clone() } }
    }
    pub fn abs(&self) -> Self { Self { neg: false, limbs: self.limbs.clone() } }

    fn trim(v: &mut Vec<u32>) { while v.last() == Some(&0) { v.pop(); } }

    fn cmp_mag(a: &[u32], b: &[u32]) -> core::cmp::Ordering {
        use core::cmp::Ordering::*;
        if a.len() != b.len() { return a.len().cmp(&b.len()); }
        for (&x, &y) in a.iter().rev().zip(b.iter().rev()) {
            match x.cmp(&y) { Equal => {} o => return o }
        }
        Equal
    }

    fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
        let mut carry = 0u64;
        for i in 0..a.len().max(b.len()) {
            let s = a.get(i).copied().unwrap_or(0) as u64
                  + b.get(i).copied().unwrap_or(0) as u64
                  + carry;
            out.push(s as u32); carry = s >> 32;
        }
        if carry != 0 { out.push(carry as u32); }
        out
    }

    fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len());
        let mut borrow = 0i64;
        for (i, _item) in a.iter().enumerate() {
            let d = a[i] as i64 - b.get(i).copied().unwrap_or(0) as i64 - borrow;
            borrow = if d < 0 { 1 } else { 0 };
            out.push((d + if d < 0 { 0x1_0000_0000 } else { 0 }) as u32);
        }
        Self::trim(&mut out); out
    }

    pub fn add(&self, other: &Self) -> Self {
        if self.neg == other.neg {
            return Self { neg: self.neg, limbs: Self::add_mag(&self.limbs, &other.limbs) };
        }
        match Self::cmp_mag(&self.limbs, &other.limbs) {
            core::cmp::Ordering::Equal => Self::zero(),
            core::cmp::Ordering::Greater => Self { neg: self.neg,  limbs: Self::sub_mag(&self.limbs,  &other.limbs) },
            core::cmp::Ordering::Less => Self { neg: other.neg, limbs: Self::sub_mag(&other.limbs, &self.limbs)  },
        }
    }
    pub fn sub(&self, other: &Self) -> Self { self.add(&other.neg()) }

    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() { return Self::zero(); }
        let (n, m) = (self.limbs.len(), other.limbs.len());
        let mut tmp = vec![0u64; n + m];
        for (i, &ai) in self.limbs.iter().enumerate() {
            for (j, &bj) in other.limbs.iter().enumerate() {
                tmp[i + j] += ai as u64 * bj as u64;
            }
        }
        let mut limbs = Vec::with_capacity(n + m);
        let mut carry = 0u64;
        for &d in &tmp { let s = d + carry; limbs.push(s as u32); carry = s >> 32; }
        if carry != 0 { limbs.push(carry as u32); }
        Self::trim(&mut limbs);
        Self { neg: self.neg != other.neg, limbs }
    }

    pub fn mul_u32(&self, d: u32) -> Self {
        if d == 0 || self.is_zero() { return Self::zero(); }
        let mut carry = 0u64;
        let mut limbs = Vec::with_capacity(self.limbs.len() + 1);
        for &l in &self.limbs { let s = l as u64 * d as u64 + carry; limbs.push(s as u32); carry = s >> 32; }
        if carry != 0 { limbs.push(carry as u32); }
        Self { neg: self.neg, limbs }
    }

    fn div_mag(u: &[u32], v: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let n = v.len();
        let m = u.len().saturating_sub(n);

        // Single-limb fast path
        if n == 1 {
            let d = v[0] as u64;
            let mut rem = 0u64;
            let mut q = vec![0u32; u.len()];
            for i in (0..u.len()).rev() {
                let cur = (rem << 32) | u[i] as u64;
                q[i] = (cur / d) as u32;
                rem = cur % d;
            }
            Self::trim(&mut q);
            return (q, if rem == 0 { vec![] } else { vec![rem as u32] });
        }

        // Normalize so v[n-1] >= BASE/2, bounding q_hat error to at most 2
        let shift = v[n - 1].leading_zeros();
        let vn = Self::shl_limbs(v, shift);
        let mut un = Self::shl_limbs_ext(u, shift);

        let (vn1, vn2) = (vn[n - 1] as u64, vn[n - 2] as u64);
        let mut q = vec![0u32; m + 1];

        for j in (0..=m).rev() {
            // Estimate quotient digit, then refine (at most 2 corrections)
            let u2 = ((un[j + n] as u64) << 32) | un[j + n - 1] as u64;
            let (mut q_hat, mut r_hat) = (u2 / vn1, u2 % vn1);
            while q_hat >= (1u64 << 32)
                || q_hat * vn2 > ((r_hat << 32) | un[j + n - 2] as u64)
            {
                q_hat -= 1;
                r_hat += vn1;
                if r_hat >= (1u64 << 32) { break; }
            }

            // Subtract q_hat * vn from un[j..]
            let mut borrow = 0i64;
            for i in 0..n {
                let prod = q_hat * vn[i] as u64;
                let diff = un[j+i] as i64 - borrow - (prod & 0xFFFF_FFFF) as i64;
                un[j+i] = diff as u32;
                borrow = (prod >> 32) as i64 - (diff >> 32);
            }
            let top = un[j + n] as i64 - borrow;
            un[j + n] = top as u32;

            // Add back if q_hat was too large (rare)
            if top < 0 {
                q_hat -= 1;
                let mut carry = 0u64;
                for i in 0..n {
                    let s = un[j+i] as u64 + vn[i] as u64 + carry;
                    un[j+i] = s as u32;
                    carry = s >> 32;
                }
                un[j+n] = un[j+n].wrapping_add(carry as u32);
            }

            q[j] = q_hat as u32;
        }

        // Denormalize remainder
        let mut rem = Self::shr_limbs(&un[..n], shift);
        Self::trim(&mut q);
        Self::trim(&mut rem);
        (q, rem)
    }

    /// Shift left, returning n+1 limbs to hold overflow.
    fn shl_limbs_ext(limbs: &[u32], shift: u32) -> Vec<u32> {
        let mut out = vec![0u32; limbs.len() + 1];
        if shift == 0 {
            out[..limbs.len()].copy_from_slice(limbs);
            return out;
        }
        out[limbs.len()] = limbs[limbs.len() - 1] >> (32 - shift);
        for i in (1..limbs.len()).rev() {
            out[i] = (limbs[i] << shift) | (limbs[i-1] >> (32 - shift));
        }
        out[0] = limbs[0] << shift;
        out
    }

    /// Shift left without overflow limb.
    fn shl_limbs(limbs: &[u32], shift: u32) -> Vec<u32> {
        if shift == 0 { return limbs.to_vec(); }
        let mut out = vec![0u32; limbs.len()];
        for i in (1..limbs.len()).rev() {
            out[i] = (limbs[i] << shift) | (limbs[i-1] >> (32 - shift));
        }
        out[0] = limbs[0] << shift;
        out
    }

    /// Shift right to undo normalization.
    fn shr_limbs(limbs: &[u32], shift: u32) -> Vec<u32> {
        if shift == 0 { return limbs.to_vec(); }
        let mut out = vec![0u32; limbs.len()];
        for i in 0..limbs.len() - 1 {
            out[i] = (limbs[i] >> shift) | (limbs[i+1] << (32 - shift));
        }
        *out.last_mut().unwrap() = limbs.last().unwrap() >> shift;
        out
    }

    pub fn divmod(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() { return None; }
        if self.is_zero()  { return Some((Self::zero(), Self::zero())); }

        let (q_l, r_l) = Self::div_mag(&self.limbs, &other.limbs);
        let mut q = Self { neg: self.neg != other.neg, limbs: q_l };
        let mut r = Self { neg: self.neg, limbs: r_l };

        if !r.is_zero() && r.neg != other.neg {
            q = q.sub(&Self { neg: false, limbs: vec![1] });
            r = r.add(other);
        }
        Some((q, r))
    }

    pub fn pow_u32(&self, mut exp: u32) -> Self {
        if exp == 0 { return Self { neg: false, limbs: vec![1] }; }
        let mut base = self.clone();
        let mut result = Self { neg: false, limbs: vec![1] };
        while exp > 0 {
            if exp & 1 != 0 { result = result.mul(&base); }
            base = base.mul(&base);
            exp >>= 1;
        }
        result
    }

    pub fn to_decimal(&self) -> alloc::string::String {
        if self.is_zero() { return alloc::string::String::from("0"); }
        const BASE: u64 = 1_000_000_000;
        let mut limbs = self.limbs.clone();
        let mut groups: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
        while !limbs.is_empty() {
            let mut rem = 0u64;
            let mut nl: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
            for &l in limbs.iter().rev() {
                let cur = (rem << 32) | l as u64;
                let q = cur / BASE; rem = cur % BASE;
                if !nl.is_empty() || q != 0 { nl.push(q as u32); }
            }
            nl.reverse(); groups.push(rem as u32); limbs = nl;
        }
        let mut s = alloc::string::String::new();
        if self.neg { s.push('-'); }
        for (i, &g) in groups.iter().rev().enumerate() {
            if i == 0 { s.push_str(&alloc::format!("{}", g)); }
            else { s.push_str(&alloc::format!("{:09}", g)); }
        }
        s
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(Ord::cmp(self, other))
    }
}
impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        if self.neg != other.neg {
            return if self.neg { core::cmp::Ordering::Less } else { core::cmp::Ordering::Greater };
        }
        let m = Self::cmp_mag(&self.limbs, &other.limbs);
        if self.neg { m.reverse() } else { m }
    }
}

/*
Heap Objects
    Str, List, Dict, Set, Tuple, Func, Range and Slice stored in arena.
*/

#[derive(Clone, Debug)]
pub enum HeapObj {
    Str(String),
    List(Rc<RefCell<Vec<Val>>>),
    Dict(Rc<RefCell<DictMap>>),
    Set(Rc<RefCell<HashSet<Val>>>),
    Tuple(Vec<Val>),
    Func(usize, Vec<Val>),
    Range(i64, i64, i64),
    Slice(Val, Val, Val),
    Type(String),
    BigInt(BigInt),
    BoundMethod(Val, BuiltinMethodId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinMethodId {
    ListAppend,
    DictKeys,
    DictValues,
    DictItems,
}

/*
DictMap
    Insertion-ordered dict backed by Vec with HashMap index for O(1) lookup.
*/

#[derive(Clone, Debug)]
pub struct DictMap {
    pub entries: Vec<(Val, Val)>,
    index: HashMap<Val, usize>,
}

impl DictMap {
    pub fn with_capacity(cap: usize) -> Self {
        Self { entries: Vec::with_capacity(cap), index: HashMap::with_capacity_and_hasher(cap, Default::default()) }
    }

    pub fn get(&self, key: &Val) -> Option<&Val> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    pub fn contains_key(&self, key: &Val) -> bool {
        self.index.contains_key(key)
    }

    pub fn insert(&mut self, key: Val, value: Val) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = value;
        } else {
            let i = self.entries.len();
            self.entries.push((key, value));
            self.index.insert(key, i);
        }
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub fn iter(&self) -> impl Iterator<Item = (Val, Val)> + '_ {
        self.entries.iter().map(|&(k, v)| (k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = Val> + '_ {
        self.entries.iter().map(|&(k, _)| k)
    }

    pub fn from_pairs(pairs: Vec<(Val, Val)>) -> Self {
        let mut dm = Self::with_capacity(pairs.len());
        for (k, v) in pairs { dm.insert(k, v); }
        dm
    }
}

impl Default for DictMap {
    fn default() -> Self { Self::new() }
}

impl DictMap {
    pub fn new() -> Self { Self { entries: Vec::new(), index: HashMap::default() } }
}

/*
Heap Pool
    Arena allocator with mark-sweep GC, string interning, and per-type tagging for inline cache.
*/

struct HeapSlot {
    obj: Option<HeapObj>,
    marked: bool,
}

pub struct HeapPool {
    slots: Vec<HeapSlot>,
    free_list: Vec<u32>,
    live: usize,
    pub gc_threshold: usize,
    alloc_count: usize,
    limit: usize,
    strings: HashMap<String, u32>,
}

impl HeapPool {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            live: 0,
            gc_threshold: 512,
            alloc_count: 0,
            limit,
            strings: HashMap::default(),
        }
    }

    pub fn alloc(&mut self, obj: HeapObj) -> Result<Val, VmErr> {
        if let HeapObj::Str(ref s) = obj
            && s.len() <= 128
            && let Some(&idx) = self.strings.get(s) {
                return Ok(Val::heap(idx));
        }
        if self.live >= self.limit { return Err(cold_heap()); }
        if self.slots.len() >= (1 << 28) { return Err(VmErr::Heap); }

        let idx = if let Some(i) = self.free_list.pop() {
            self.slots[i as usize] = HeapSlot { obj: Some(obj), marked: false };
            i
        } else {
            let i = self.slots.len() as u32;
            self.slots.push(HeapSlot { obj: Some(obj), marked: false });
            i
        };

        if let HeapObj::Str(s) = self.slots[idx as usize].obj.as_ref().unwrap()
            && s.len() <= 128 { self.strings.insert(s.clone(), idx); }

        self.live += 1;
        self.alloc_count += 1;
        Ok(Val::heap(idx))
    }

    pub fn mark(&mut self, v: Val) {
        if !v.is_heap() { return; }
        let mut worklist = vec![v.as_heap()];
        while let Some(idx) = worklist.pop() {
            let idx = idx as usize;
            if self.slots[idx].marked { continue; }
            self.slots[idx].marked = true;
            match &self.slots[idx].obj {
                Some(HeapObj::Tuple(items)) => {
                    for v in items { if v.is_heap() { worklist.push(v.as_heap()); } }
                }
                Some(HeapObj::Slice(a,b,c)) => { for v in [*a,*b,*c] { if v.is_heap() { worklist.push(v.as_heap()); } } }
                Some(HeapObj::List(rc)) => worklist.extend(rc.borrow().iter().filter(|v| v.is_heap()).map(|v| v.as_heap())),
                Some(HeapObj::Dict(rc)) => worklist.extend(rc.borrow().entries.iter().flat_map(|(k,v)| [*k,*v]).filter(|v| v.is_heap()).map(|v| v.as_heap())),
                Some(HeapObj::Set(rc)) => {
                    for v in rc.borrow().iter() {
                        if v.is_heap() {
                            worklist.push(v.as_heap());
                        }
                    }
                }
                Some(HeapObj::BoundMethod(recv, _))
                    if recv.is_heap() => { worklist.push(recv.as_heap()); }
                _ => {}
            }
        }
    }

    pub fn sweep(&mut self) {
        for idx in 0..self.slots.len() {
            let slot = &mut self.slots[idx];
            match &slot.obj {
                None => {}
                Some(_) if slot.marked => { slot.marked = false; }
                Some(HeapObj::Str(s)) => {
                    self.strings.remove(s);
                    slot.obj = None;
                    self.free_list.push(idx as u32);
                    self.live -= 1;
                }
                Some(_) => {
                    slot.obj = None;
                    self.free_list.push(idx as u32);
                    self.live -= 1;
                }
            }
        }

        self.gc_threshold = (self.live * 2).max(512);
        self.alloc_count  = 0;

        // Cap free list at 512K slots; sort to prefer low indices and reduce fragmentation.
        if self.free_list.len() > 524_288 {
            self.free_list.sort_unstable();
            self.free_list.truncate(524_288);
        }
    }

    pub fn needs_gc(&self) -> bool {
        let alloc_limit = (self.live / 4).max(4096);
        self.live >= self.gc_threshold || self.alloc_count >= alloc_limit
    }

    pub fn usage(&self) -> usize { self.live }

    #[inline(always)] pub fn get(&self, v: Val) -> &HeapObj {
        self.slots[v.as_heap() as usize].obj
            .as_ref()
            .expect("garbage collector invariant violated: live Val references a freed heap slot")
    }
    #[inline(always)] pub fn get_mut(&mut self, v: Val) -> &mut HeapObj {
        self.slots[v.as_heap() as usize].obj
            .as_mut()
            .expect("garbage collector invariant violated: live Val references a freed heap slot (mut)")
    }
    

    #[inline(always)]
        pub fn val_tag(&self, v: Val) -> u8 {
            if v.is_int() { 1 } else if v.is_float() { 2 } else if v.is_bool() { 3 }
            else if v.is_none() { 4 } else if v.is_heap() {
                match self.slots[v.as_heap() as usize].obj.as_ref() {
                    Some(HeapObj::Str(_)) => 5,
                    Some(HeapObj::List(_)) => 6,
                    Some(HeapObj::Dict(_)) => 7,
                    Some(HeapObj::Set(_)) => 8,
                    Some(HeapObj::Tuple(_)) => 9,
                    Some(HeapObj::Func(_, _)) => 10,
                    Some(HeapObj::Range(..)) => 11,
                    Some(HeapObj::Slice(..)) => 12,
                    Some(HeapObj::Type(_)) => 13,
                    Some(HeapObj::BigInt(_)) => 14,
                    Some(HeapObj::BoundMethod(_, _)) => 15,
                    None => 0,
                }
            } else { 0 }
        }
    }

/*
Deep Value Equality
    Content-based equality over the heap; canonical implementation used by both.
*/

pub(super) fn eq_seq(a: &[Val], b: &[Val], eq: impl Fn(Val,Val)->bool) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x,y)| eq(*x,*y))
}
pub(super) fn eq_dict(a: &DictMap, b: &DictMap, eq: impl Fn(Val,Val)->bool) -> bool {
    a.len() == b.len() && a.iter().all(|(k,v)| b.get(&k).is_some_and(|&v2| eq(v,v2)))
}

pub fn eq_vals_with_heap(a: Val, b: Val, heap: &HeapPool) -> bool {
    if let (Some(ba), Some(bb)) = (bigint_of(a, heap), bigint_of(b, heap)) {
        return ba.cmp(bb) == core::cmp::Ordering::Equal;
    }

    if !a.is_heap() || !b.is_heap() {
        if a.is_int() && b.is_int() { return a.as_int() == b.as_int(); }
        if a.is_float() && b.is_float() { return a.as_float() == b.as_float(); }
        if a.is_int() && b.is_float() { return (a.as_int() as f64) == b.as_float(); }
        if a.is_float() && b.is_int() { return a.as_float() == (b.as_int() as f64); }
        return a.0 == b.0;
    }

    match (heap.get(a), heap.get(b)) {
        (HeapObj::BigInt(x), HeapObj::BigInt(y)) => x.cmp(y) == core::cmp::Ordering::Equal,
        (HeapObj::Str(x), HeapObj::Str(y)) => x == y,
        (HeapObj::Tuple(x), HeapObj::Tuple(y)) => eq_seq(x, y, |a,b| eq_vals_with_heap(a, b, heap)),
        (HeapObj::List(x), HeapObj::List(y)) => eq_seq(&x.borrow(), &y.borrow(), |a,b| eq_vals_with_heap(a, b, heap)),
        (HeapObj::Set(x), HeapObj::Set(y)) => *x.borrow() == *y.borrow(),
        (HeapObj::Dict(x), HeapObj::Dict(y)) => eq_dict(&x.borrow(), &y.borrow(), |a,b| eq_vals_with_heap(a, b, heap)),
        _ => false,
    }
}

fn bigint_of(v: Val, heap: &HeapPool) -> Option<&BigInt> {
    if v.is_heap() && let HeapObj::BigInt(b) = heap.get(v) { return Some(b); }
    None
}

/*
Runtime Errors
    CallDepth, Heap, Budget, Name, Type, Value, ZeroDiv and Runtime variants.
*/

pub enum VmErr {
    CallDepth, Heap, Budget, ZeroDiv,
    Name(String),
    Type(&'static str),
    Value(&'static str),
    Runtime(&'static str),
    Raised(String),
}

impl fmt::Display for VmErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallDepth => write!(f, "RecursionError: max depth"),
            Self::Heap => write!(f, "MemoryError: heap limit"),
            Self::Budget => write!(f, "RuntimeError: budget exceeded"),
            Self::ZeroDiv => write!(f, "ZeroDivisionError: division by zero"),
            Self::Name(s) => write!(f, "NameError: '{}'", s),
            Self::Type(s) => write!(f, "TypeError: {}", s),
            Self::Value(s) => write!(f, "ValueError: {}", s),
            Self::Runtime(s) => write!(f, "RuntimeError: {}", s),
            Self::Raised(s) => write!(f, "Exception: {}", s),
        }
    }
}

/*
Iterator Frame
    Seq or Range state consumed one item at a time by ForIter dispatch.
*/

pub enum IterFrame {
    Seq { items: Vec<Val>, idx: usize },
    Range { cur: i64, end: i64, step: i64 }
}

impl IterFrame {
    pub fn next_item(&mut self) -> Option<Val> {
        match self {
            Self::Seq { items, idx } => {
                if *idx < items.len() { let v = items[*idx]; *idx += 1; Some(v) } else { None }
            }
            Self::Range { cur, end, step } => {
                let done = if *step > 0 { *cur >= *end } else { *cur <= *end };
                if done { None } else { let v = *cur; *cur += *step; Some(Val::int(v)) }
            }
        }
    }
}

/*
Math Helpers
    Pure f64 implementations of powi, round, powf for no_std and WASM builds.
*/

#[inline]
pub fn fpowi(mut base: f64, exp: i32) -> f64 {
    if exp == 0 { return 1.0; }
    let neg = exp < 0;
    let mut e = (exp as i64).unsigned_abs() as u32;
    let mut r = 1.0;
    while e > 0 { if e & 1 != 0 { r *= base; } base *= base; e >>= 1; }
    if neg { 1.0 / r } else { r }
}

#[inline]
pub fn fround(x: f64) -> f64 {
    let i = x as i64;
    let t = i as f64;
    let d = x - t;
    if d > 0.5 { t + 1.0 }
    else if d < -0.5 { t - 1.0 }
    else if d == 0.5 { if i % 2 == 0 { t } else { t + 1.0 } }
    else if d == -0.5 { if i % 2 == 0 { t } else { t - 1.0 } }
    else { t }
}

pub fn fln(x: f64) -> f64 {
    let bits = f64::to_bits(x);
    let exp = ((bits >> 52) & 0x7FF) as i64 - 1023;
    let m = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
    let t = (m - 1.0) / (m + 1.0); let t2 = t * t;
    2.0 * t * (1.0 + t2 * (1.0/3.0 + t2 * (1.0/5.0 + t2 * (1.0/7.0 + t2 / 9.0)))) + exp as f64 * core::f64::consts::LN_2
}

pub fn fexp(x: f64) -> f64 {
    if x > 709.0 { return f64::INFINITY; }
    if x < -709.0 { return 0.0; }
    let k = (x * core::f64::consts::LOG2_E) as i64;
    let r = x - k as f64 * core::f64::consts::LN_2;
    let e = 1.0 + r * (1.0 + r * (0.5 + r * (1.0/6.0 + r * (1.0/24.0 + r * (1.0/120.0 + r / 720.0)))));
    f64::from_bits(((k + 1023) as u64) << 52) * e
}

#[inline]
pub fn fpowf(base: f64, exp: f64) -> f64 {
    let ei = exp as i32;
    if (ei as f64) == exp { return fpowi(base, ei); }
    if base <= 0.0 {
        if base == 0.0 { return if exp > 0.0 { 0.0 } else { f64::INFINITY }; }
        return f64::NAN;
    }
    fexp(exp * fln(base))
}

/*
Cold Error Constructors
    Out-of-line error paths keep hot dispatch loop linear for instruction cache.
*/

#[cold] #[inline(never)] pub fn cold_heap() -> VmErr { VmErr::Heap }
#[cold] #[inline(never)] pub fn cold_budget() -> VmErr { VmErr::Budget }
#[cold] #[inline(never)] pub fn cold_depth() -> VmErr { VmErr::CallDepth }