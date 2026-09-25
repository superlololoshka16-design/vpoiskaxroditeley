#![allow(clippy::missing_safety_doc)]

use hmac::SimpleHmac;
use md5::Digest as _;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
use std::sync::OnceLock;

use crate::BytesExt as _;

pub const PBKDF2_MAX_ITERATIONS: u32 = 10_000_000;
pub const SUBTLE_MAX_OUT_BYTES: usize = 1_048_576;

#[inline]
pub fn sha256_into(data: &[u8], out: &mut [u8; 32]) {
    *out = sha2::Sha256::digest(data).into();
}

#[inline]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    sha256_into(data, &mut out);
    out
}

pub fn sha256_midstate(mut prefix: [u32; 8], data: &[u8]) -> ([u32; 8], [u8; 63], usize) {
    let nfull = data.len() / 64;
    for i in 0..nfull {
        let blk: &[u8; 64] = data[i * 64..i * 64 + 64].try_into().unwrap();
        sha256_block(&mut prefix, blk);
    }
    let tail_len = data.len() % 64;
    let mut tail = [0u8; 63];
    tail[..tail_len].copy_from_slice(&data[data.len() - tail_len..]);
    (prefix, tail, tail_len)
}

#[inline]
pub fn sha1_into(data: &[u8], out: &mut [u8; 20]) {
    *out = sha1::Sha1::digest(data).into();
}

#[inline]
pub fn sha256_hex_into(data: &[u8], out: &mut [u8; 64]) {
    let d = sha2::Sha256::digest(data);
    d.hex_lower_into(out);
}

#[inline]
pub fn md5_hex_into(data: &[u8], out: &mut [u8; 32]) {
    let d: [u8; 16] = md5::Md5::digest(data).into();
    d.hex_lower_into(out);
}

#[inline]
pub fn hmac_sha256_into(key: &[u8], data: &[u8], out: &mut [u8; 32]) {
    use hmac::digest::{KeyInit, Mac as _};
    type HmacSha256 = SimpleHmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    let fin = mac.finalize();
    let bytes: &[u8] = &fin.into_bytes();
    out.copy_from_slice(&bytes[..32]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfError {
    Iterations,
    Length,
}

pub fn pbkdf2_sha256_into(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    out: &mut [u8],
) -> Result<(), KdfError> {
    if iterations == 0 || iterations > PBKDF2_MAX_ITERATIONS {
        return Err(KdfError::Iterations);
    }
    if out.is_empty() || out.len() > SUBTLE_MAX_OUT_BYTES {
        return Err(KdfError::Length);
    }
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, out);
    Ok(())
}

#[inline]
pub fn lz_carry_chain(pairs: impl Iterator<Item = (u32, bool)>) -> u32 {
    let mut acc = 0u32;
    let mut carry = 1u32;
    for (lz, zero) in pairs {
        acc += carry * lz;
        carry &= u32::from(zero);
        if carry == 0 {
            break;
        }
    }
    acc
}

#[inline]
pub fn lz_words_be(words: &[u32]) -> u32 {
    let mut acc = 0u32;
    let mut carry = 1u32;
    for &w in words.iter().take(8) {
        let lz = w.leading_zeros();
        acc += carry * lz;
        carry &= u32::from(w == 0);
    }
    acc
}

#[inline]
pub fn be32_words(digest: &[u8; 32]) -> [u32; 8] {
    std::array::from_fn(|i| digest.be_u32(i * 4))
}

#[inline]
pub fn words_be32(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (c, &w) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *c = w.to_be_bytes();
    }
    out
}

pub fn sha256_seed_tail(seed: u64, tail: &[u8]) -> [u8; 32] {
    let mut h = sha2::Sha256::new();
    h.update(seed.to_le_bytes());
    h.update(tail);
    h.finalize().into()
}

const BLAKE2B_IV: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

const BLAKE2B_SIGMA: [[u8; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

pub const BLAKE2B_G_COLS: [[u8; 4]; 8] = [
    [0, 4, 8, 12],
    [1, 5, 9, 13],
    [2, 6, 10, 14],
    [3, 7, 11, 15],
    [0, 5, 10, 15],
    [1, 6, 11, 12],
    [2, 7, 8, 13],
    [3, 4, 9, 14],
];

#[macro_export]
macro_rules! blake2b_g_col {
    ($v:expr, $a:expr, $b:expr, $c:expr, $d:expr, $x:expr, $y:expr, $f:expr) => {{
        $v[$a] = $f($v[$a], $v[$b], $x);
        $v[$d] = ($v[$d] ^ $v[$a]).rotate_right(32);
        $v[$c] = $f($v[$c], $v[$d], 0);
        $v[$b] = ($v[$b] ^ $v[$c]).rotate_right(24);
        $v[$a] = $f($v[$a], $v[$b], $y);
        $v[$d] = ($v[$d] ^ $v[$a]).rotate_right(16);
        $v[$c] = $f($v[$c], $v[$d], 0);
        $v[$b] = ($v[$b] ^ $v[$c]).rotate_right(63);
    }};
}

#[inline(always)]
fn blake2b_g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    crate::blake2b_g_col!(v, a, b, c, d, x, y, |p: u64, q: u64, m: u64| {
        p.wrapping_add(q).wrapping_add(m)
    });
}

#[inline(always)]
fn blake2b_compress(h: &mut [u64; 8], m: &[u64; 16], t0: u64, t1: u64, f0: u64, f1: u64) {
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&BLAKE2B_IV);
    v[12] ^= t0;
    v[13] ^= t1;
    v[14] ^= f0;
    v[15] ^= f1;
    for s in &BLAKE2B_SIGMA {
        blake2b_g(&mut v, 0, 4, 8, 12, m[s[0] as usize], m[s[1] as usize]);
        blake2b_g(&mut v, 1, 5, 9, 13, m[s[2] as usize], m[s[3] as usize]);
        blake2b_g(&mut v, 2, 6, 10, 14, m[s[4] as usize], m[s[5] as usize]);
        blake2b_g(&mut v, 3, 7, 11, 15, m[s[6] as usize], m[s[7] as usize]);
        blake2b_g(&mut v, 0, 5, 10, 15, m[s[8] as usize], m[s[9] as usize]);
        blake2b_g(&mut v, 1, 6, 11, 12, m[s[10] as usize], m[s[11] as usize]);
        blake2b_g(&mut v, 2, 7, 8, 13, m[s[12] as usize], m[s[13] as usize]);
        blake2b_g(&mut v, 3, 4, 9, 14, m[s[14] as usize], m[s[15] as usize]);
    }
    for (i, hi) in h.iter_mut().enumerate() {
        *hi ^= v[i] ^ v[i + 8];
    }
}

#[inline(always)]
fn le16_words(buf: &[u8; 128]) -> [u64; 16] {
    std::array::from_fn(|k| u64::from_le_bytes(buf[k * 8..k * 8 + 8].try_into().unwrap()))
}

struct Blake2bHasher {
    h: [u64; 8],
    buf: [u8; 128],
    buflen: usize,
    counter: u128,
}

impl Blake2bHasher {
    fn new(outlen: usize) -> Self {
        assert!(
            (1..=64).contains(&outlen),
            "blake2b outlen must be 1..=64, got {outlen}"
        );
        let mut param = [0u8; 64];
        param[0] = outlen as u8;
        param[2] = 1;
        param[3] = 1;
        let h: [u64; 8] = std::array::from_fn(|i| {
            BLAKE2B_IV[i] ^ u64::from_le_bytes(param[i * 8..i * 8 + 8].try_into().unwrap())
        });
        Self {
            h,
            buf: [0u8; 128],
            buflen: 0,
            counter: 0,
        }
    }

    #[inline]
    fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let take = (128 - self.buflen).min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            data = &data[take..];
            if self.buflen == 128 {
                self.counter += 128;
                let m = le16_words(&self.buf);
                blake2b_compress(
                    &mut self.h,
                    &m,
                    self.counter as u64,
                    (self.counter >> 64) as u64,
                    0,
                    0,
                );
                self.buflen = 0;
            }
        }
    }

    fn finish(&mut self, out: &mut [u8]) {
        self.counter += self.buflen as u128;
        self.buf[self.buflen..].fill(0);
        let m = le16_words(&self.buf);
        blake2b_compress(
            &mut self.h,
            &m,
            self.counter as u64,
            (self.counter >> 64) as u64,
            u64::MAX,
            0,
        );
        let mut full = [0u8; 64];
        for (c, w) in full.as_chunks_mut::<8>().0.iter_mut().zip(self.h) {
            *c = w.to_le_bytes();
        }
        out.copy_from_slice(&full[..out.len()]);
    }
}

pub fn blake2b(out: &mut [u8], outlen: usize, input: &[u8]) {
    assert!(
        out.len() >= outlen,
        "blake2b out buffer ({}) < outlen ({})",
        out.len(),
        outlen
    );
    let mut st = Blake2bHasher::new(outlen);
    st.update(input);
    st.finish(&mut out[..outlen]);
}

pub fn blake2b_long(out: &mut [u8], input: &[u8]) {
    let outlen = out.len();
    let outlen_le = (outlen as u32).to_le_bytes();

    if outlen <= 64 {
        let mut st = Blake2bHasher::new(outlen);
        st.update(&outlen_le);
        st.update(input);
        st.finish(out);
        return;
    }
    let mut out_buffer = [0u8; 64];
    {
        let mut st = Blake2bHasher::new(64);
        st.update(&outlen_le);
        st.update(input);
        st.finish(&mut out_buffer);
    }
    out[..32].copy_from_slice(&out_buffer[..32]);
    let mut produced = 32usize;
    let mut toproduce = outlen - 32;
    while toproduce > 64 {
        let mut ob = [0u8; 64];
        blake2b(&mut ob, 64, &out_buffer);
        out[produced..produced + 32].copy_from_slice(&ob[..32]);
        out_buffer = ob;
        produced += 32;
        toproduce -= 32;
    }
    let mut ob = [0u8; 64];
    blake2b(&mut ob, toproduce, &out_buffer);
    out[produced..produced + toproduce].copy_from_slice(&ob[..toproduce]);
}

const ADLER_MOD: u32 = 65521;
const ADLER_NMAX: usize = 5552;

#[inline]
fn adler32_block16(data: &[u8], a: &mut u32, b: &mut u32) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        if cpu_avx2() {
            let v = _mm_loadu_si128(data.as_ptr() as *const __m128i);
            let zero = _mm_setzero_si128();
            let sad = _mm_sad_epu8(v, zero);
            let mut sad_lanes = [0u64; 2];
            _mm_storeu_si128(sad_lanes.as_mut_ptr() as *mut __m128i, sad);
            let s = sad_lanes[0] + sad_lanes[1];
            let wv = _mm_setr_epi8(16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1);
            let wm = _mm_maddubs_epi16(v, wv);
            let w32 = _mm_madd_epi16(wm, _mm_set1_epi16(1));
            let mut wl = [0i32; 4];
            _mm_storeu_si128(wl.as_mut_ptr() as *mut __m128i, w32);
            let w = (wl[0] + wl[1] + wl[2] + wl[3]) as u64;
            let la = u64::from(*a);
            *a = (la + s) as u32;
            *b = (u64::from(*b) + 16 * la + w) as u32;
            return;
        }
    }
    {
        let (mut la, mut lb) = (u64::from(*a), u64::from(*b));
        for k in 0..16 {
            la += u64::from(data[k]);
            lb += la;
        }
        *a = la as u32;
        *b = lb as u32;
    }
}


#[inline]
pub fn adler32_feed(state: u32, data: &[u8]) -> u32 {
    let (mut a, mut b) = (state & 0xFFFF, state >> 16);
    for block in data.chunks(ADLER_NMAX) {
        let mut i = 0usize;
        while i + 16 <= block.len() {
            adler32_block16(&block[i..i + 16], &mut a, &mut b);
            i += 16;
        }
        for &byte in &block[i..] {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

#[inline]
pub fn adler32_zeros(state: u32, n: usize) -> u32 {
    if n == 0 {
        return state;
    }
    let a = u64::from(state & 0xFFFF);
    let b = u64::from(state >> 16);
    let b_final = ((b + a * n as u64) % u64::from(ADLER_MOD)) as u32;
    ((b_final << 16) | (state & 0xFFFF))
}

const fn crc_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut s = 1usize;
    while s < 8 {
        let mut i = 0usize;
        while i < 256 {
            t[s][i] = (t[s - 1][i] >> 8) ^ t[0][(t[s - 1][i] & 0xFF) as usize];
            i += 1;
        }
        s += 1;
    }
    t
}

static CRC_TABLES: [[u32; 256]; 8] = crc_tables();

#[inline(always)]
fn crc_fold8(c: u32, hi: u32) -> u32 {
    let t = &CRC_TABLES;
    t[7][(c & 0xFF) as usize]
        ^ t[6][((c >> 8) & 0xFF) as usize]
        ^ t[5][((c >> 16) & 0xFF) as usize]
        ^ t[4][(c >> 24) as usize]
        ^ t[3][(hi & 0xFF) as usize]
        ^ t[2][((hi >> 8) & 0xFF) as usize]
        ^ t[1][((hi >> 16) & 0xFF) as usize]
        ^ t[0][(hi >> 24) as usize]
}

#[inline(always)]
fn crc_byte(c: u32, b: u8) -> u32 {
    (c >> 8) ^ CRC_TABLES[0][((c ^ u32::from(b)) & 0xFF) as usize]
}

pub fn crc32_feed(mut c: u32, data: &[u8]) -> u32 {
    let (chunks, rest) = data.as_chunks::<8>();
    for block in chunks {
        let lo = u32::from_le_bytes(*block.first_chunk().unwrap());
        let hi = u32::from_le_bytes(*block.last_chunk().unwrap());
        c ^= lo;
        c = crc_fold8(c, hi);
    }
    for &b in rest {
        c = crc_byte(c, b);
    }
    c
}
pub const K32: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[cfg(target_arch = "x86_64")]
fn cpuid_ebx7_snapshot() -> u32 {
    use std::arch::x86_64::__cpuid_count;
    static EBX7: OnceLock<u32> = OnceLock::new();
    *EBX7.get_or_init(|| __cpuid_count(7, 0).ebx)
}

#[cfg(not(target_arch = "x86_64"))]
fn cpuid_ebx7_snapshot() -> u32 {
    0
}

#[cfg(target_arch = "x86_64")]
static SHA_NI: OnceLock<bool> = OnceLock::new();

#[inline]
pub fn cpu_sha() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        *SHA_NI.get_or_init(|| (cpuid_ebx7_snapshot() >> 29) & 1 == 1)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[inline]
pub fn cpu_avx512cd() -> bool {
    (cpuid_ebx7_snapshot() >> 28) & 1 == 1
}

#[inline]
pub fn cpu_avx2() -> bool {
    (cpuid_ebx7_snapshot() >> 5) & 1 == 1
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn vzero_barrier() {
    unsafe { std::arch::asm!("vzeroupper", options(nostack, preserves_flags)) };
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn load_be(p: *const u8, mask: __m128i) -> __m128i {
    unsafe { _mm_shuffle_epi8(_mm_loadu_si128(p as *const __m128i), mask) }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn state_in(state: *const u32) -> (__m128i, __m128i) {
    unsafe {
        let dcba = _mm_loadu_si128(state as *const __m128i);
        let efgh = _mm_loadu_si128(state.add(4) as *const __m128i);
        let cdab = _mm_shuffle_epi32(dcba, 0xB1);
        let efgh = _mm_shuffle_epi32(efgh, 0x1B);
        let abef = _mm_alignr_epi8(cdab, efgh, 8);
        let cdgh = _mm_blend_epi16(efgh, cdab, 0xF0);
        (abef, cdgh)
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn state_out(
    state: *mut u32,
    mut abef: __m128i,
    mut cdgh: __m128i,
    abef_save: __m128i,
    cdgh_save: __m128i,
) {
    unsafe {
        abef = _mm_add_epi32(abef, abef_save);
        cdgh = _mm_add_epi32(cdgh, cdgh_save);
        let feba = _mm_shuffle_epi32(abef, 0x1B);
        let dchg = _mm_shuffle_epi32(cdgh, 0xB1);
        let dcba = _mm_blend_epi16(feba, dchg, 0xF0);
        let hgef = _mm_alignr_epi8(dchg, feba, 8);
        _mm_storeu_si128(state as *mut __m128i, dcba);
        _mm_storeu_si128(state.add(4) as *mut __m128i, hgef);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn schedule(v0: __m128i, v1: __m128i, v2: __m128i, v3: __m128i) -> __m128i {
    unsafe {
        let t1 = _mm_sha256msg1_epu32(v0, v1);
        let t2 = _mm_alignr_epi8(v3, v2, 4);
        let t3 = _mm_add_epi32(t1, t2);
        _mm_sha256msg2_epu32(t3, v3)
    }
}

macro_rules! rounds4 {
    ($abef:ident, $cdgh:ident, $rest:expr, $i:expr) => {{
        let kv = _mm_set_epi32(
            K32[($i) * 4 + 3] as i32,
            K32[($i) * 4 + 2] as i32,
            K32[($i) * 4 + 1] as i32,
            K32[($i) * 4] as i32,
        );
        let t1 = _mm_add_epi32($rest, kv);
        $cdgh = _mm_sha256rnds2_epu32($cdgh, $abef, t1);
        let t2 = _mm_shuffle_epi32(t1, 0x0E);
        $abef = _mm_sha256rnds2_epu32($abef, $cdgh, t2);
    }};
}

macro_rules! schedule_rounds4 {
    ($abef:ident, $cdgh:ident, $w0:expr, $w1:expr, $w2:expr, $w3:expr, $w4:expr, $i:expr) => {{
        $w4 = schedule($w0, $w1, $w2, $w3);
        rounds4!($abef, $cdgh, $w4, $i);
    }};
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
pub unsafe fn sha256_block_ni(state: &mut [u32; 8], block: &[u8; 64]) {
    unsafe {
        let mask: __m128i = _mm_set_epi64x(
            0x0C0D_0E0F_0809_0A0Bu64 as i64,
            0x0405_0607_0001_0203u64 as i64,
        );
        let dp = block.as_ptr();
        let sp = state.as_ptr();
        let (mut abef, mut cdgh) = state_in(sp);
        let abef_save = abef;
        let cdgh_save = cdgh;
        let mut w0 = load_be(dp, mask);
        let mut w1 = load_be(dp.add(16), mask);
        let mut w2 = load_be(dp.add(32), mask);
        let mut w3 = load_be(dp.add(48), mask);
        let mut w4;
        rounds4!(abef, cdgh, w0, 0);
        rounds4!(abef, cdgh, w1, 1);
        rounds4!(abef, cdgh, w2, 2);
        rounds4!(abef, cdgh, w3, 3);
        schedule_rounds4!(abef, cdgh, w0, w1, w2, w3, w4, 4);
        schedule_rounds4!(abef, cdgh, w1, w2, w3, w4, w0, 5);
        schedule_rounds4!(abef, cdgh, w2, w3, w4, w0, w1, 6);
        schedule_rounds4!(abef, cdgh, w3, w4, w0, w1, w2, 7);
        schedule_rounds4!(abef, cdgh, w4, w0, w1, w2, w3, 8);
        schedule_rounds4!(abef, cdgh, w0, w1, w2, w3, w4, 9);
        schedule_rounds4!(abef, cdgh, w1, w2, w3, w4, w0, 10);
        schedule_rounds4!(abef, cdgh, w2, w3, w4, w0, w1, 11);
        schedule_rounds4!(abef, cdgh, w3, w4, w0, w1, w2, 12);
        schedule_rounds4!(abef, cdgh, w4, w0, w1, w2, w3, 13);
        schedule_rounds4!(abef, cdgh, w0, w1, w2, w3, w4, 14);
        schedule_rounds4!(abef, cdgh, w1, w2, w3, w4, w0, 15);
        let spm = state.as_mut_ptr();
        state_out(spm, abef, cdgh, abef_save, cdgh_save);

        vzero_barrier();
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
pub unsafe fn sha256_block_ni_2x(
    state0: &mut [u32; 8],
    state1: &mut [u32; 8],
    block0: &[u8; 64],
    block1: &[u8; 64],
) {
    unsafe {
        vzero_barrier();
        let mask: __m128i = _mm_set_epi64x(
            0x0C0D_0E0F_0809_0A0Bu64 as i64,
            0x0405_0607_0001_0203u64 as i64,
        );
        let d0 = block0.as_ptr();
        let d1 = block1.as_ptr();
        let s0p = state0.as_ptr();
        let s1p = state1.as_ptr();
        let (mut a0, mut c0) = state_in(s0p);
        let (mut a1, mut c1) = state_in(s1p);
        let a0_save = a0;
        let c0_save = c0;
        let a1_save = a1;
        let c1_save = c1;
        let mut w00 = load_be(d0, mask);
        let mut w01 = load_be(d0.add(16), mask);
        let mut w02 = load_be(d0.add(32), mask);
        let mut w03 = load_be(d0.add(48), mask);
        let mut w10 = load_be(d1, mask);
        let mut w11 = load_be(d1.add(16), mask);
        let mut w12 = load_be(d1.add(32), mask);
        let mut w13 = load_be(d1.add(48), mask);
        let mut w04;
        let mut w14;
        rounds4!(a0, c0, w00, 0);
        rounds4!(a1, c1, w10, 0);
        rounds4!(a0, c0, w01, 1);
        rounds4!(a1, c1, w11, 1);
        rounds4!(a0, c0, w02, 2);
        rounds4!(a1, c1, w12, 2);
        rounds4!(a0, c0, w03, 3);
        rounds4!(a1, c1, w13, 3);
        schedule_rounds4!(a0, c0, w00, w01, w02, w03, w04, 4);
        schedule_rounds4!(a1, c1, w10, w11, w12, w13, w14, 4);
        schedule_rounds4!(a0, c0, w01, w02, w03, w04, w00, 5);
        schedule_rounds4!(a1, c1, w11, w12, w13, w14, w10, 5);
        schedule_rounds4!(a0, c0, w02, w03, w04, w00, w01, 6);
        schedule_rounds4!(a1, c1, w12, w13, w14, w10, w11, 6);
        schedule_rounds4!(a0, c0, w03, w04, w00, w01, w02, 7);
        schedule_rounds4!(a1, c1, w13, w14, w10, w11, w12, 7);
        schedule_rounds4!(a0, c0, w04, w00, w01, w02, w03, 8);
        schedule_rounds4!(a1, c1, w14, w10, w11, w12, w13, 8);
        schedule_rounds4!(a0, c0, w00, w01, w02, w03, w04, 9);
        schedule_rounds4!(a1, c1, w10, w11, w12, w13, w14, 9);
        schedule_rounds4!(a0, c0, w01, w02, w03, w04, w00, 10);
        schedule_rounds4!(a1, c1, w11, w12, w13, w14, w10, 10);
        schedule_rounds4!(a0, c0, w02, w03, w04, w00, w01, 11);
        schedule_rounds4!(a1, c1, w12, w13, w14, w10, w11, 11);
        schedule_rounds4!(a0, c0, w03, w04, w00, w01, w02, 12);
        schedule_rounds4!(a1, c1, w13, w14, w10, w11, w12, 12);
        schedule_rounds4!(a0, c0, w04, w00, w01, w02, w03, 13);
        schedule_rounds4!(a1, c1, w14, w10, w11, w12, w13, 13);
        schedule_rounds4!(a0, c0, w00, w01, w02, w03, w04, 14);
        schedule_rounds4!(a1, c1, w10, w11, w12, w13, w14, 14);
        schedule_rounds4!(a0, c0, w01, w02, w03, w04, w00, 15);
        schedule_rounds4!(a1, c1, w11, w12, w13, w14, w10, 15);
        let s0m = state0.as_mut_ptr();
        let s1m = state1.as_mut_ptr();
        state_out(s0m, a0, c0, a0_save, c0_save);
        state_out(s1m, a1, c1, a1_save, c1_save);
        vzero_barrier();
    }
}

#[inline(always)]
pub fn pair_mut(st: &mut [[u32; 8]; 8], i: usize) -> (&mut [u32; 8], &mut [u32; 8]) {
    unsafe {
        let p = st.as_mut_ptr();
        (&mut *p.add(i), &mut *p.add(i + 1))
    }
}


#[inline]
pub fn state_of(digest: &[u8; 32]) -> [u32; 8] {
    be32_words(digest)
}

#[inline]
pub fn digest_bytes(state: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

pub fn sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    #[cfg(target_arch = "x86_64")]
    {
        if cpu_sha() {
            unsafe { sha256_block_ni(state, block) };
            return;
        }
    }
    sha2::block_api::compress256(state, std::slice::from_ref(block));
}

#[inline(always)]
pub fn compress8(st: &mut [[u32; 8]; 8], blocks: &[[u8; 64]; 8]) {
    #[cfg(target_arch = "x86_64")]
    {
        if cpu_sha() {
            unsafe {
                for k in 0..4 {
                    let a = k * 2;
                    let (sa, sb) = pair_mut(st, a);
                    sha256_block_ni_2x(sa, sb, &blocks[a], &blocks[a + 1]);
                }
            }
            return;
        }
    }
    for (s, b) in st.iter_mut().zip(blocks.iter()) {
        sha2::block_api::compress256(s, std::slice::from_ref(b));
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512cd")]
pub unsafe fn lz_batch16(words: &[u32; 16]) -> [u32; 2] {
    unsafe {
        let v = _mm512_loadu_si512(words.as_ptr() as *const __m512i);
        let lz = _mm512_lzcnt_epi32(v);
        let zm = _mm512_cmpeq_epi32_mask(v, _mm512_setzero_si512()) as u16;
        let mut lz_arr = [0u32; 16];
        _mm512_storeu_si512(lz_arr.as_mut_ptr() as *mut __m512i, lz);
        let mut out = [0u32; 2];
        for (g, o) in out.iter_mut().enumerate() {
            let zero_mask = (zm >> (g * 8)) as u8;
            *o = lz_carry_chain(
                lz_arr[g * 8..g * 8 + 8]
                    .iter()
                    .enumerate()
                    .map(|(i, &lzi)| (lzi, zero_mask >> i & 1 == 1)),
            );
        }
        vzero_barrier();
        out
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512cd")]
pub unsafe fn lz_batch16_ref(w0: &[u32; 8], w1: &[u32; 8]) -> [u32; 2] {
    unsafe {
        let a = _mm256_loadu_si256(w0.as_ptr() as *const __m256i);
        let b = _mm256_loadu_si256(w1.as_ptr() as *const __m256i);
        let v = _mm512_inserti64x4(_mm512_castsi256_si512(a), b, 1);
        let lz = _mm512_lzcnt_epi32(v);
        let zm = _mm512_cmpeq_epi32_mask(v, _mm512_setzero_si512()) as u16;
        let mut lz_arr = [0u32; 16];
        _mm512_storeu_si512(lz_arr.as_mut_ptr() as *mut __m512i, lz);
        let mut out = [0u32; 2];
        for (g, o) in out.iter_mut().enumerate() {
            let zero_mask = (zm >> (g * 8)) as u8;
            *o = lz_carry_chain(
                lz_arr[g * 8..g * 8 + 8]
                    .iter()
                    .enumerate()
                    .map(|(i, &lzi)| (lzi, zero_mask >> i & 1 == 1)),
            );
        }
        vzero_barrier();
        out
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn lz_batch16_ref(w0: &[u32; 8], w1: &[u32; 8]) -> [u32; 2] {
    [lz_words_be(w0), lz_words_be(w1)]
}

#[inline]
pub(crate) fn xxh3_seed_tail(seed: u64, tail: &[u8]) -> u64 {
    use std::hash::Hasher as _;
    let mut h = crate::xxh3::XxHash3_64::new();
    h.write(&seed.to_le_bytes());
    h.write(tail);
    h.finish()
}


#[inline]
pub fn sha_tail_pad(dst: &mut [u8], at: usize, bit_len: u64) {
    dst[at] = 0x80;
    let end = if at < 56 { 64 } else { 128 };
    dst[at + 1..end - 8].fill(0);
    dst[end - 8..end].copy_from_slice(&bit_len.to_be_bytes());
}

