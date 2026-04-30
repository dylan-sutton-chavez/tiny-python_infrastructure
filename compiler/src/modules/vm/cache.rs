// vm/cache.rs

use super::types::{Val, eq_vals_with_heap};
use crate::modules::parser::{OpCode, SSAChunk, Instruction};
use crate::modules::fx::FxHashMap as HashMap;

use alloc::{vec, vec::Vec};

/* Specialized operation types for inline cache type-stable binary dispatch. */

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

/* Per-instruction IC slot. */

const QUICK_THRESH: u8 = 4;

#[derive(Clone, Default)]
struct CacheSlot {
    type_key: u8,
    hits: u8,
    fast: Option<FastOp>,
}

pub struct OpcodeCache {
    slots: Vec<CacheSlot>,
    fused: Option<Vec<Instruction>>,
}

impl OpcodeCache {
    pub fn new(chunk: &SSAChunk) -> Self {
        Self {
            slots: vec![CacheSlot::default(); chunk.instructions.len()],
            fused: None,
        }
    }

    /// Compile fused instruction stream on first access; reuse afterward.
    pub fn ensure_fused(&mut self, chunk: &SSAChunk) -> &[Instruction] {
        if self.fused.is_none() {
            self.fused = Some(fuse_method_calls(chunk));
        }
        self.fused.as_ref().unwrap()
    }

    /// Direct access (caller must have called `ensure_fused`).
    pub fn fused_ref(&self) -> &[Instruction] {
        self.fused.as_ref().expect("fused code not compiled")
    }

    pub fn record(&mut self, ip: usize, opcode: &OpCode, ta: u8, tb: u8) {
        let Some(s) = self.slots.get_mut(ip) else { return };
        let key = (ta << 4) | (tb & 0xF);
        if s.type_key == key {
            s.hits = s.hits.saturating_add(1);
            if s.hits >= QUICK_THRESH && s.fast.is_none() {
                s.fast = Self::specialize(opcode, ta, tb);
            }
        } else {
            *s = CacheSlot { type_key: key, hits: 1, fast: None };
        }
    }

    #[inline]
    pub fn get_fast(&self, ip: usize) -> Option<FastOp> {
        self.slots.get(ip).and_then(|s| s.fast)
    }

    pub fn invalidate(&mut self, ip: usize) {
        if let Some(s) = self.slots.get_mut(ip) { *s = CacheSlot::default(); }
    }

    fn specialize(opcode: &OpCode, ta: u8, tb: u8) -> Option<FastOp> {
        match (opcode, ta, tb) {
            (OpCode::Add, 1, 1) => Some(FastOp::AddInt),    (OpCode::Add, 2, 2) => Some(FastOp::AddFloat),
            (OpCode::Add, 5, 5) => Some(FastOp::AddStr),    (OpCode::Sub, 1, 1) => Some(FastOp::SubInt),
            (OpCode::Sub, 2, 2) => Some(FastOp::SubFloat),  (OpCode::Mul, 1, 1) => Some(FastOp::MulInt),
            (OpCode::Mul, 2, 2) => Some(FastOp::MulFloat),  (OpCode::Lt, 1, 1) => Some(FastOp::LtInt),
            (OpCode::Lt, 2, 2) => Some(FastOp::LtFloat),    (OpCode::Eq, 1, 1) => Some(FastOp::EqInt),
            (OpCode::Eq, 5, 5) => Some(FastOp::EqStr),      (OpCode::Gt, 1, 1) => Some(FastOp::GtInt),
            (OpCode::LtEq, 1, 1) => Some(FastOp::LtEqInt),  (OpCode::GtEq, 1, 1) => Some(FastOp::GtEqInt),
            (OpCode::NotEq, 1, 1) => Some(FastOp::NotEqInt), _ => None,
        }
    }
}

/* Template memoization — unchanged */

fn args_match(e: &TplEntry, args: &[Val], h: u64, heap: &super::types::HeapPool) -> bool {
    e.hash == h
    && e.args.len() == args.len()
    && e.args.iter().zip(args).all(|(a, b)| eq_vals_with_heap(*a, *b, heap))
}

const TPL_THRESH: u32 = 2;

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

/// Fuses LoadAttr+Call into CallMethod+CallMethodArgs in-place. Replaces
/// the two opcodes but keeps both operands and instruction count, so jump
/// targets stay valid. Compiled once per chunk; cached above.
fn fuse_method_calls(chunk: &SSAChunk) -> Vec<Instruction> {
    let src = &chunk.instructions;
    let n = src.len();
    let mut out = src.clone();

    let mut i = 0;
    while i + 1 < n {
        if src[i].opcode == OpCode::LoadAttr && src[i + 1].opcode == OpCode::Call {
            out[i].opcode = OpCode::CallMethod;
            out[i + 1].opcode = OpCode::CallMethodArgs;
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}