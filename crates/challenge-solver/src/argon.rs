use std::cell::RefCell;
use std::cell::UnsafeCell;

use core_utils::blake2b;
use core_utils::blake2b_long;
use smallvec::SmallVec;
const PREFETCH_MIN_BYTES: u32 = 1024 * 1024;

#[inline(always)]
fn fbla_mka(x: u64, y: u64) -> u64 {
    let m = 0xFFFF_FFFFu64;
    x.wrapping_add(y)
        .wrapping_add(((x & m).wrapping_mul(y & m)) << 1)
}

#[inline(always)]
fn g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize) {
    core_utils::blake2b_g_col!(v, a, b, c, d, 0, 0, |p: u64, q: u64, _m: u64| fbla_mka(
        p, q
    ));
}

#[inline(always)]
fn round_nomsg(v: &mut [u64; 16]) {
    for &[a, b, c, d] in &core_utils::BLAKE2B_G_COLS {
        g(v, a as usize, b as usize, c as usize, d as usize);
    }
}

#[inline(always)]
fn permute(r: &mut [u64; 128]) {
    for k in 0..8usize {
        let row: &mut [u64; 16] = (&mut r[k * 16..k * 16 + 16]).try_into().unwrap();
        round_nomsg(row);
    }
    for i in 0..8usize {
        let mut v = [0u64; 16];
        for c in 0..8 {
            for j in 0..2 {
                v[2 * c + j] = r[2 * i + 16 * c + j];
            }
        }
        round_nomsg(&mut v);
        for c in 0..8 {
            for j in 0..2 {
                r[2 * i + 16 * c + j] = v[2 * c + j];
            }
        }
    }
}

#[inline(always)]
fn fill_block(mem: &mut [[u64; 128]], prev: usize, rf: usize, curr: usize, with_xor: bool) {
    let mut r = [0u64; 128];
    for ((rk, &a), &b) in r.iter_mut().zip(&mem[rf]).zip(&mem[prev]) {
        *rk = a ^ b;
    }
    let mut tmp = r;
    if with_xor {
        for (tk, &c) in tmp.iter_mut().zip(&mem[curr]) {
            *tk ^= c;
        }
    }
    permute(&mut r);
    for ((mk, &t), &p) in mem[curr].iter_mut().zip(&tmp).zip(&r) {
        *mk = t ^ p;
    }
}

#[inline(always)]
fn next_addresses(address: &mut [u64; 128], input: &mut [u64; 128]) {
    input[6] = input[6].wrapping_add(1);
    let mut r = *input;
    permute(&mut r);
    for ((ak, &rk), &ik) in address.iter_mut().zip(&r).zip(input.iter()) {
        *ak = rk ^ ik;
    }
    let old = *address;
    let mut r2 = old;
    permute(&mut r2);
    for (ak, (&rk, &ok)) in address.iter_mut().zip(r2.iter().zip(&old)) {
        *ak = rk ^ ok;
    }
}

#[derive(Clone, Copy)]
struct Geo {
    lane_length: u32,
    segment_length: u32,
    m_prime: u32,
    t_cost: u32,
    y: u32,
    lane_div: core_utils::math::FastDivU64,
}

#[derive(Clone, Copy)]
struct RefPos {
    pass: u32,
    slice: u32,
    index: u32,
    pseudo_rand: u32,
    same_lane: bool,
}

#[inline(always)]
fn index_alpha(geo: &Geo, pos: RefPos) -> u32 {
    let RefPos {
        pass,
        slice,
        index,
        pseudo_rand,
        same_lane,
    } = pos;
    let segment_length = geo.segment_length;
    let lane_length = geo.lane_length;
    let ref_area: u32;
    if pass == 0 {
        if slice == 0 {
            ref_area = index.wrapping_sub(1);
        } else if same_lane {
            ref_area = slice
                .wrapping_mul(segment_length)
                .wrapping_add(index)
                .wrapping_sub(1);
        } else {
            ref_area = slice
                .wrapping_mul(segment_length)
                .wrapping_add(u32::from(index == 0) * u32::MAX);
        }
    } else if same_lane {
        ref_area = lane_length
            .wrapping_sub(segment_length)
            .wrapping_add(index)
            .wrapping_sub(1);
    } else {
        ref_area = lane_length
            .wrapping_sub(segment_length)
            .wrapping_add(u32::from(index == 0) * u32::MAX);
    }
    let mut rel: u64 = pseudo_rand as u64;
    rel = (rel.wrapping_mul(rel)) >> 32;
    rel = (ref_area as u64)
        .wrapping_sub(1)
        .wrapping_sub(((ref_area as u64).wrapping_mul(rel)) >> 32);
    let start: u32 = if pass != 0 {
        if slice == 3 {
            0
        } else {
            (slice + 1) * segment_length
        }
    } else {
        0
    };
    geo.lane_div.rem((start as u64).wrapping_add(rel)) as u32
}

#[inline(always)]
fn prefetch_t0(p: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch(p as *const i8, std::arch::x86_64::_MM_HINT_T0);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = p;
}

struct Filler<'a> {
    mem: &'a mut [[u64; 128]],
    geo: Geo,
}

impl Filler<'_> {
    fn segment(&mut self, pass: u32, slice: u32, lane: u32) {
        let geo = self.geo;
        let lane_length = geo.lane_length;
        let segment_length = geo.segment_length;
        let data_independent = geo.y == 1 || (geo.y == 2 && pass == 0 && slice < 2);
        let mut input = [0u64; 128];
        let mut address = [0u64; 128];
        if data_independent {
            input[0] = pass as u64;
            input[1] = lane as u64;
            input[2] = slice as u64;
            input[3] = geo.m_prime as u64;
            input[4] = geo.t_cost as u64;
            input[5] = geo.y as u64;
        }
        let mut starting_index = 0u32;
        if pass == 0 && slice == 0 {
            starting_index = 2;
            if data_independent {
                next_addresses(&mut address, &mut input);
            }
        }
        let mut curr_offset = lane * lane_length + slice * segment_length + starting_index;
        let mut lane_rem = curr_offset % lane_length;
        let mut prev_offset = if curr_offset.is_multiple_of(lane_length) {
            curr_offset + lane_length - 1
        } else {
            curr_offset - 1
        };
        let mut i = starting_index;
        while i < segment_length {
            if lane_rem == 1 {
                prev_offset = curr_offset - 1;
            }
            let pseudo_rand: u64 = if data_independent {
                if i.is_multiple_of(128) {
                    next_addresses(&mut address, &mut input);
                }
                address[(i % 128) as usize]
            } else {
                self.mem[prev_offset as usize][0]
            };
            let mut ref_lane = 0u32;
            if pass == 0 && slice == 0 {
                ref_lane = lane;
            }
            let ref_index = index_alpha(
                &geo,
                RefPos {
                    pass,
                    slice,
                    index: i,
                    pseudo_rand: (pseudo_rand & 0xFFFF_FFFF) as u32,
                    same_lane: ref_lane == lane,
                },
            );
            let ref_off = (ref_lane * lane_length + ref_index) as usize;
            if geo.m_prime * 1024 > PREFETCH_MIN_BYTES {
                unsafe {
                    prefetch_t0(self.mem[ref_off].as_ptr() as *const u8);
                    prefetch_t0(self.mem[ref_off].as_ptr().add(512) as *const u8);
                }
            }
            fill_block(
                self.mem,
                prev_offset as usize,
                ref_off,
                curr_offset as usize,
                pass != 0,
            );
            curr_offset += 1;
            prev_offset += 1;
            lane_rem = if lane_rem + 1 == lane_length { 0 } else { lane_rem + 1 };
            i += 1;
        }
    }
}

pub struct ArgonCtx {
    salt: SmallVec<[u8; 192]>,
    m_cost: u32,
    t_cost: u32,
    m_prime: u32,
    need_bits: u32,
}

impl ArgonCtx {
    pub fn new(salt: &[u8], difficulty: u8, m_cost: u32, t_cost: u32) -> Self {
        const MAX_MEM_COST: u32 = 64 * 1024;
        let m_cost = m_cost.clamp(8, MAX_MEM_COST);
        ArgonCtx {
            salt: SmallVec::from_slice(salt),
            m_cost,
            t_cost: t_cost.max(1),
            m_prime: 4 * (m_cost / 4),
            need_bits: crate::scan::need_bits_of(difficulty),
        }
    }

    pub fn m_prime(&self) -> u32 {
        self.m_prime
    }

    pub fn digest(&self, nonce: u64, width: usize, mem: &mut [[u64; 128]]) -> [u8; 32] {
        let pw_len = self.salt.len() + width;
        let mut h0_input = [0u8; 512];
        let mut w = 0usize;
        let mut digits = [0u8; 20];
        crate::scan::digits_of(nonce, &mut digits, width);
        let put = |h: &mut [u8; 512], w: &mut usize, bytes: &[u8]| {
            h[*w..*w + bytes.len()].copy_from_slice(bytes);
            *w += bytes.len();
        };
        put(&mut h0_input, &mut w, &1u32.to_le_bytes());
        put(&mut h0_input, &mut w, &32u32.to_le_bytes());
        put(&mut h0_input, &mut w, &self.m_cost.to_le_bytes());
        put(&mut h0_input, &mut w, &self.t_cost.to_le_bytes());
        put(&mut h0_input, &mut w, &0x13u32.to_le_bytes());
        put(&mut h0_input, &mut w, &2u32.to_le_bytes());
        put(&mut h0_input, &mut w, &(pw_len as u32).to_le_bytes());
        put(&mut h0_input, &mut w, &self.salt);
        put(&mut h0_input, &mut w, &digits[..width]);
        put(&mut h0_input, &mut w, &(self.salt.len() as u32).to_le_bytes());
        put(&mut h0_input, &mut w, &self.salt);
        put(&mut h0_input, &mut w, &0u32.to_le_bytes());
        put(&mut h0_input, &mut w, &0u32.to_le_bytes());
        let mut h0 = [0u8; 64];
        blake2b(&mut h0, 64, &h0_input[..w]);

        let mut blockhash = [0u8; 72];
        blockhash[..64].copy_from_slice(&h0);

        let mut blk = [0u8; 1024];
        blockhash[64..68].copy_from_slice(&0u32.to_le_bytes());
        blockhash[68..72].copy_from_slice(&0u32.to_le_bytes());
        blake2b_long(&mut blk, &blockhash);
        for (m, c) in mem[0].iter_mut().zip(blk.as_chunks::<8>().0) {
            *m = u64::from_le_bytes(*c);
        }
        blockhash[64..68].copy_from_slice(&1u32.to_le_bytes());
        blake2b_long(&mut blk, &blockhash);
        for (m, c) in mem[1].iter_mut().zip(blk.as_chunks::<8>().0) {
            *m = u64::from_le_bytes(*c);
        }

        let geo = Geo {
            lane_length: self.m_prime,
            segment_length: self.m_prime / 4,
            m_prime: self.m_prime,
            t_cost: self.t_cost,
            y: 2,
            lane_div: core_utils::math::FastDivU64::new(u64::from(self.m_prime)),
        };
        let mut filler = Filler { mem, geo };
        for pass in 0..geo.t_cost {
            for slice in 0..4u32 {
                filler.segment(pass, slice, 0);
            }
        }

        let mut cbytes = [0u8; 1024];
        let c = &filler.mem[(geo.m_prime - 1) as usize];
        for (cb, &w) in cbytes.as_chunks_mut::<8>().0.iter_mut().zip(c.iter()) {
            *cb = w.to_le_bytes();
        }
        let mut tag = [0u8; 32];
        blake2b_long(&mut tag, &cbytes);
        tag
    }
}

struct Arena {
    map: UnsafeCell<memmap2::MmapMut>,
    len: usize,
}

impl Arena {
    fn new(blocks: usize) -> Option<Self> {
        let bytes = blocks.checked_mul(1024)?;
        let map = if bytes >= 2 * 1024 * 1024 {
            memmap2::MmapOptions::new()
                .len(bytes)
                .huge(Some(21))
                .map_anon()
                .or_else(|_| {
                    memmap2::MmapOptions::new()
                        .len(bytes)
                        .map_anon()
                        .map(|mut m| {
                            advise_hugepage(&mut m);
                            m
                        })
                })
        } else {
            memmap2::MmapOptions::new().len(bytes).map_anon()
        };
        let map = map.ok()?;
        Some(Arena {
            map: UnsafeCell::new(map),
            len: blocks,
        })
    }

    fn as_blocks(&mut self) -> &mut [[u64; 128]] {
        let map = unsafe { &mut *self.map.get() };
        unsafe { std::slice::from_raw_parts_mut(map.as_mut_ptr() as *mut [u64; 128], self.len) }
    }

    fn blocks(&self) -> usize {
        self.len
    }
}

#[cfg(target_os = "linux")]
fn advise_hugepage(map: &mut memmap2::MmapMut) {
    unsafe {
        unsafe extern "C" {
            fn madvise(addr: *mut core::ffi::c_void, len: usize, advice: i32) -> i32;
        }
        const MADV_HUGEPAGE: i32 = 14;
        let _ = madvise(
            map.as_mut_ptr() as *mut core::ffi::c_void,
            map.len(),
            MADV_HUGEPAGE,
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_hugepage(_map: &mut memmap2::MmapMut) {}

thread_local! {
    static ARENA: RefCell<Option<Arena>> = const { RefCell::new(None) };
}

const ARGON_CHUNK_CAP: u64 = 256;

fn with_arena<R>(m_prime: usize, f: impl FnOnce(&mut [[u64; 128]]) -> R) -> Option<R> {
    ARENA.with(|a| {
        let mut guard = a.borrow_mut();
        if guard.as_ref().is_none_or(|ar| ar.blocks() < m_prime) {
            *guard = Arena::new(m_prime);
        }
        let arena = guard.as_mut()?;
        let mem = arena.as_blocks();
        Some(f(mem))
    })
}

pub fn solve(
    salt: &[u8],
    difficulty: u8,
    m_cost: u32,
    t_cost: u32,
    threads: usize,
) -> Option<(u64, [u8; 32])> {
    solve_until(
        salt,
        difficulty,
        m_cost,
        t_cost,
        threads,
        &std::sync::atomic::AtomicBool::new(false),
    )
}

pub fn solve_until(
    salt: &[u8],
    difficulty: u8,
    m_cost: u32,
    t_cost: u32,
    threads: usize,
    abort: &std::sync::atomic::AtomicBool,
) -> Option<(u64, [u8; 32])> {
    let ctx = ArgonCtx::new(salt, difficulty, m_cost, t_cost);
    let ctx = &ctx;
    let m_prime = ctx.m_prime as usize;
    let need_bits = ctx.need_bits;
    crate::scan::width_solve_until(threads, 12, 8, 1, ARGON_CHUNK_CAP, move |width, b, end| {
        with_arena(m_prime, |mem| {
            crate::scan::scalar_scan(b, end, need_bits, |n| ctx.digest(n, width, mem))
        })
        .flatten()
    }, abort)
}
