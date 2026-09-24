use core_utils::crypto::{H0, sha256_midstate};

use crate::scan::{BatchCtx, ScanPlan, tail_digest};

pub struct PowCtx {
    prefix: [u32; 8],
    tail: [u8; 63],
    tail_len: usize,
    data_len: usize,
    need_bits: u32,
    avx512: bool,
}

impl PowCtx {
    pub fn new(data: &[u8], difficulty: u8) -> Self {
        let (prefix, tail, tail_len) = sha256_midstate(H0, data);
        PowCtx {
            prefix,
            tail,
            tail_len,
            data_len: data.len(),
            need_bits: crate::scan::need_bits_of(difficulty),
            avx512: crate::scan::cpu_avx512_enabled(),
        }
    }

    pub fn tail_len(&self) -> usize {
        self.tail_len
    }

    pub fn digest(&self, nonce: u64, width: usize) -> [u8; 32] {
        tail_digest(
            self.prefix,
            &self.tail[..self.tail_len],
            self.data_len + width,
            nonce,
            width,
        )
    }
}

impl BatchCtx for PowCtx {
    fn plan_of(&self, width: usize) -> ScanPlan {
        ScanPlan::build(width, &self.tail[..self.tail_len], self.data_len + width)
    }

    fn batch_off(&self) -> usize {
        self.tail_len
    }

    fn batch_prefix(&self) -> [u32; 8] {
        self.prefix
    }

    fn need_bits(&self) -> u32 {
        self.need_bits
    }

    fn avx512(&self) -> bool {
        self.avx512
    }

    unsafe fn mid_step(&self, _st: &mut [[u32; 8]; 8], _blocks: &mut [[u8; 64]; 8]) {}
}

pub fn solve(data: &[u8], difficulty: u8, threads: usize) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve(&PowCtx::new(data, difficulty), threads)
}

pub fn solve_until(
    data: &[u8],
    difficulty: u8,
    threads: usize,
    abort: &std::sync::atomic::AtomicBool,
) -> Option<(u64, [u8; 32])> {
    crate::scan::batched_solve_until(&PowCtx::new(data, difficulty), threads, abort)
}
