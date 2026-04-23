// src/modules/fx.rs

//! Multiply-rotate hasher for small integer/string keys.

use core::hash::{BuildHasher, Hasher};
use core::sync::atomic::{AtomicUsize, Ordering};

const K: u64 = 0x517cc1b727220a95;

static SEED_COUNTER: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Default)]
pub struct FxHasher(u64);

impl Hasher for FxHasher {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(buf)).wrapping_mul(K);
        }
    }
    #[inline(always)] fn write_u8(&mut self, i: u8) { self.write_u64(i as u64); }
    #[inline(always)] fn write_u16(&mut self, i: u16) { self.write_u64(i as u64); }
    #[inline(always)] fn write_u32(&mut self, i: u32) { self.write_u64(i as u64); }
    #[inline(always)] fn write_u64(&mut self, i: u64) { self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(K); }
    #[inline(always)] fn write_usize(&mut self, i: usize) { self.write_u64(i as u64); }
    #[inline(always)] fn finish(&self) -> u64 { self.0 }
}

#[derive(Clone)]
pub struct FxBuildHasher(u64);

impl FxBuildHasher {
    /// Atomic accumulator XORed with stack ptr, mixed with K. Racy store avoids contention. Seed is per-map.
    #[inline]
    pub fn new() -> Self {
        let prev = SEED_COUNTER.load(Ordering::Relaxed) as u64;
        let stack_ptr = &prev as *const u64 as u64;
        let seed = (prev ^ stack_ptr).wrapping_mul(K);
        SEED_COUNTER.store(seed as usize, Ordering::Relaxed);
        Self(seed)
    }
}

impl Default for FxBuildHasher {
    fn default() -> Self { Self::new() }
}

impl BuildHasher for FxBuildHasher {
    type Hasher = FxHasher;
    #[inline(always)]
    fn build_hasher(&self) -> FxHasher { FxHasher(self.0) }
}

pub type FxHashMap<K, V> = hashbrown::HashMap<K, V, FxBuildHasher>;
pub type FxHashSet<T> = hashbrown::HashSet<T, FxBuildHasher>;