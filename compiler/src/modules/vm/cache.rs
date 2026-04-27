// vm/cache.rs

use super::types::{Val, eq_vals_with_heap};
use super::super_ops::{self, SuperOp};
use crate::modules::parser::{OpCode, SSAChunk};
use alloc::{vec, vec::Vec};
use crate::modules::fx::FxHashMap as HashMap;

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
    GtInt, LtEqInt, GtEqInt,
    EqInt, EqStr,
    NotEqInt
}

/*
Opcode Cache
    Per-frame slot combining inline-cache type recording with adaptive hot-path rewriting. Collocates both tiers per instruction to avoid split cache lines.
*/

const CACHE_THRESH: u8 = 3;
const HOT_THRESH: u32 = 517;

#[derive(Clone, Default)]
struct CacheSlot {
    ic_hits: u8, ta: u8, tb: u8,
    ic_fast: Option<FastOp>,
    hot_count: u32,
    hot_fast: Option<FastOp>,
}

pub struct OpcodeCache {
    slots: Vec<CacheSlot>,
    super_ops: Vec<Option<SuperOp>>,
}

impl OpcodeCache {
    pub fn new(chunk: &SSAChunk) -> Self {
        Self {
            slots: vec![CacheSlot::default(); chunk.instructions.len()],
            super_ops: super_ops::detect(chunk),
        }
    }

    #[inline] pub fn get_super(&self, ip: usize) -> Option<SuperOp> {
        self.super_ops.get(ip).copied().flatten()
    }

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
            *s = CacheSlot { ta, tb, ic_hits: 1, ..CacheSlot::default() };
        }
    }

    #[inline] pub fn get_fast(&self, ip: usize) -> Option<FastOp> {
        self.slots.get(ip).and_then(|s| s.hot_fast.or(s.ic_fast))
    }

    pub fn invalidate(&mut self, ip: usize) {
        if let Some(s) = self.slots.get_mut(ip) { *s = CacheSlot::default(); }
    }

    fn specialize(opcode: &OpCode, ta: u8, tb: u8) -> Option<FastOp> {
        match (opcode, ta, tb) {
            (OpCode::Add, 1, 1) => Some(FastOp::AddInt), (OpCode::Add, 2, 2) => Some(FastOp::AddFloat),
            (OpCode::Add, 5, 5) => Some(FastOp::AddStr), (OpCode::Sub, 1, 1) => Some(FastOp::SubInt),
            (OpCode::Sub, 2, 2) => Some(FastOp::SubFloat), (OpCode::Mul, 1, 1) => Some(FastOp::MulInt),
            (OpCode::Mul, 2, 2) => Some(FastOp::MulFloat), (OpCode::Lt, 1, 1) => Some(FastOp::LtInt),
            (OpCode::Lt, 2, 2) => Some(FastOp::LtFloat), (OpCode::Eq, 1, 1) => Some(FastOp::EqInt),
            (OpCode::Eq, 5, 5) => Some(FastOp::EqStr), (OpCode::Gt, 1, 1) => Some(FastOp::GtInt),
            (OpCode::LtEq, 1, 1) => Some(FastOp::LtEqInt), (OpCode::GtEq, 1, 1) => Some(FastOp::GtEqInt),
            (OpCode::NotEq, 1, 1) => Some(FastOp::NotEqInt), _ => None,
        }
    }
}

/*
Template Memoization
    Caches pure function results by deep-equal argument matching after four repeated calls.
*/

fn args_match(e: &TplEntry, args: &[Val], h: u64, heap: &super::types::HeapPool) -> bool {
    e.hash == h
    && e.args.len() == args.len()
    && e.args.iter().zip(args).all(|(a, b)| eq_vals_with_heap(*a, *b, heap))
}

const TPL_THRESH: u32 = 2; // `is_pure` eliminate risk using two for threshold, making memoization safe for production.

struct TplEntry { args: Vec<Val>, result: Val, hits: u32, hash: u64 }

fn hash_args(args: &[Val]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in args {
        h ^= v.0;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub struct Templates { map: HashMap<usize, Vec<TplEntry>> }

impl Templates {
    pub fn new() -> Self { Self { map: HashMap::default() } }

    pub fn lookup(&self, fi: usize, args: &[Val], heap: &super::types::HeapPool) -> Option<Val> {
        let h = hash_args(args);
        self.map.get(&fi)?.iter()
            .find(|e| e.hits >= TPL_THRESH && args_match(e, args, h, heap))
            .map(|e| e.result)
    }

    pub fn record(&mut self, fi: usize, args: &[Val], result: Val, heap: &super::types::HeapPool) {
        let h = hash_args(args);
        let v = self.map.entry(fi).or_default();
        if let Some(e) = v.iter_mut().find(|e| args_match(e, args, h, heap)) {
            e.hits += 1; e.result = result;
        } else if v.len() < 256 {
            v.push(TplEntry { args: args.to_vec(), result, hits: 1, hash: h });
        }
    }

    pub fn count(&self) -> usize {
        self.map.values().flat_map(|v| v.iter()).filter(|e| e.hits >= TPL_THRESH).count()
    }
}