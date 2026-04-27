// vm/handlers/mod.rs

pub(crate) mod arith;
pub(crate) mod data;
pub(crate) mod function;
pub(crate) mod unsupported;
pub(crate) mod attr;

pub(super) use crate::modules::vm::{
    VM, Val, VmErr, HeapObj, DictMap, cache, ops,
    types::{BigInt, cold_depth, eq_vals_with_heap, fpowi, fpowf}
};

pub(super) use crate::modules::parser::{OpCode, SSAChunk};
pub(super) use crate::modules::fx::FxHashMap as HashMap;
pub(super) use alloc::{rc::Rc, string::String, vec, vec::Vec};
pub(super) use core::cell::RefCell;