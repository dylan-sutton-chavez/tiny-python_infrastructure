// vm/cache.rs
use super::types::{Val, eq_vals_deep};
use crate::modules::parser::OpCode;
use alloc::{vec, vec::Vec};
use hashbrown::HashMap;

/*
FastOp Variants
    Specialized operation types for inline cache type-stable binary dispatch.
*/

#[derive(Debug, Clone, Copy)]
pub enum FastOp {
    AddInt, AddFloat, AddStr,
    SubInt, SubFloat,
    MulInt, MulFloat,
    LtInt, LtFloat,
    EqInt, EqStr,
}

/*
Opcode Cache
    Per-frame slot combining inline-cache type recording with adaptive hot-path rewriting. Collocates both tiers per instruction to avoid split cache lines.
*/

const CACHE_THRESH: u8 = 8;
const HOT_THRESH: u32 = 1_000;

#[derive(Clone)]
struct CacheSlot {
    ic_hits: u8, ta: u8, tb: u8,
    ic_fast: Option<FastOp>,
    hot_count: u32,
    hot_fast: Option<FastOp>,
}
impl CacheSlot { fn empty() -> Self { Self { ic_hits: 0, ta: 0, tb: 0, ic_fast: None, hot_count: 0, hot_fast: None } } }

pub struct OpcodeCache { slots: Vec<CacheSlot> }

impl OpcodeCache {
    pub fn new(n: usize) -> Self { Self { slots: vec![CacheSlot::empty(); n] } }

    pub fn record(&mut self, ip: usize, opcode: &OpCode, ta: u8, tb: u8) {
        let Some(s) = self.slots.get_mut(ip) else { return };
        if s.ta == ta && s.tb == tb {
            s.ic_hits = s.ic_hits.saturating_add(1);
            if s.ic_hits >= CACHE_THRESH && s.ic_fast.is_none() {
                s.ic_fast = Self::specialize(opcode, ta, tb);
            }
            if s.ic_fast.is_some() {
                s.hot_count += 1;
                if s.hot_count == HOT_THRESH { s.hot_fast = s.ic_fast; }
            }
        } else {
            *s = CacheSlot { ta, tb, ic_hits: 1, ..CacheSlot::empty() };
        }
    }

    #[inline] pub fn get_fast(&self, ip: usize) -> Option<FastOp> {
        self.slots.get(ip).and_then(|s| s.hot_fast.or(s.ic_fast))
    }

    pub fn invalidate(&mut self, ip: usize) {
        if let Some(s) = self.slots.get_mut(ip) { *s = CacheSlot::empty(); }
    }

    pub fn specialized_count(&self) -> usize {
        self.slots.iter().filter(|s| s.hot_fast.is_some()).count()
    }

    fn specialize(opcode: &OpCode, ta: u8, tb: u8) -> Option<FastOp> {
        match (opcode, ta, tb) {
            (OpCode::Add, 1, 1) => Some(FastOp::AddInt), (OpCode::Add, 2, 2) => Some(FastOp::AddFloat),
            (OpCode::Add, 5, 5) => Some(FastOp::AddStr), (OpCode::Sub, 1, 1) => Some(FastOp::SubInt),
            (OpCode::Sub, 2, 2) => Some(FastOp::SubFloat), (OpCode::Mul, 1, 1) => Some(FastOp::MulInt),
            (OpCode::Mul, 2, 2) => Some(FastOp::MulFloat), (OpCode::Lt, 1, 1) => Some(FastOp::LtInt),
            (OpCode::Lt, 2, 2) => Some(FastOp::LtFloat), (OpCode::Eq, 1, 1) => Some(FastOp::EqInt),
            (OpCode::Eq, 5, 5) => Some(FastOp::EqStr), _ => None,
        }
    }
}

/*
Template Memoization
    Caches pure function results by deep-equal argument matching after four repeated calls.
*/

const TPL_THRESH: u32 = 4;

struct TplEntry { args: Vec<Val>, result: Val, hits: u32 }

pub struct Templates { map: HashMap<usize, Vec<TplEntry>> }

impl Templates {
    pub fn new() -> Self { Self { map: HashMap::new() } }

    pub fn lookup(&self, fi: usize, args: &[Val], heap: &super::types::HeapPool) -> Option<Val> {
        self.map.get(&fi)?.iter()
            .find(|e| {
                e.hits >= TPL_THRESH
                && e.args.len() == args.len()
                && e.args.iter().zip(args).all(|(a, b)| eq_vals_deep(*a, *b, heap))
            })
            .map(|e| e.result)
    }

    pub fn record(&mut self, fi: usize, args: &[Val], result: Val, heap: &super::types::HeapPool) {
        let v = self.map.entry(fi).or_default();
        if let Some(e) = v.iter_mut().find(|e| {
            e.args.len() == args.len()
            && e.args.iter().zip(args).all(|(a, b)| eq_vals_deep(*a, *b, heap))
        }) {
            e.hits += 1; e.result = result;
        } else if v.len() < 256 {
            v.push(TplEntry { args: args.to_vec(), result, hits: 1 });
        }
    }

    pub fn count(&self) -> usize {
        self.map.values().flat_map(|v| v.iter()).filter(|e| e.hits >= TPL_THRESH).count()
    }
}