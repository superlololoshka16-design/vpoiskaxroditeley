use compact_str::CompactString;
use smallvec::SmallVec;

pub use base64_turbo::STANDARD as B64_STANDARD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum B64Error {
    Malformed,
    TooLarge(usize),
}

pub trait BytesExt {
    fn find_sub(&self, needle: &[u8]) -> Option<usize>;
    fn find_quoted(&self, key: &[u8]) -> Option<&Self>;
    fn find_u32(&self, key: &[u8]) -> Option<u32>;
    fn find_token(&self, key: &[u8], stop: u8, max_len: usize) -> Option<&Self>;
    fn hex_lower_into(&self, out: &mut [u8]);
    fn b64_decode_into(&self, out: &mut [u8]) -> Result<usize, B64Error>;
    fn b64_encode_into(&self, out: &mut [u8]) -> Result<usize, B64Error>;
    fn b64_string(&self) -> String;
    fn b64_field_decode(&self, out: &mut SmallVec<[u8; 4096]>, max_out: usize) -> Result<usize, B64Error>;
    fn trim_ascii_extra(&self, extra: &[u8]) -> &Self;
    fn eq_ci(&self, other: &[u8]) -> bool;
    fn starts_with_ci(&self, prefix: &[u8]) -> bool;
    fn ends_with_ci(&self, suffix: &[u8]) -> bool;
    fn contains_word(&self, word: &[u8]) -> bool;
    fn contains_ci(&self, needle: &[u8]) -> bool;
    fn strip_prefix_ci(&self, prefix: &[u8]) -> Option<&Self>;
    fn leb128(&self, pos: &mut usize) -> Option<u64>;
}

impl BytesExt for [u8] {
    #[inline]
    fn find_sub(&self, needle: &[u8]) -> Option<usize> {
        memchr::memmem::find(self, needle)
    }

    fn find_quoted(&self, key: &[u8]) -> Option<&Self> {
        let rel = self.find_sub(key)?;
        let start = rel + key.len();
        if start >= self.len() {
            return None;
        }
        let mut i = start;
        while i < self.len() {
            match self[i] {
                b'\\' => i += 2,
                b'"' => {
                    return if i > start {
                        Some(&self[start..i])
                    } else {
                        None
                    };
                }
                _ => i += 1,
            }
        }
        None
    }

    fn find_u32(&self, key: &[u8]) -> Option<u32> {
        let mut from = 0usize;
        while let Some(rel) = self[from..].find_sub(key) {
            let mut i = from + rel + key.len();
            let mut val: u32 = 0;
            let mut digits = 0;
            while i < self.len() && self[i].is_ascii_digit() && digits < 8 {
                val = val * 10 + u32::from(self[i] - b'0');
                i += 1;
                digits += 1;
            }
            if digits > 0 {
                return Some(val);
            }
            from = from + rel + 1;
        }
        None
    }

    fn find_token(&self, key: &[u8], stop: u8, max_len: usize) -> Option<&Self> {
        let pos = self.find_sub(key)?;
        let start = pos + key.len();
        let mut i = start;
        while i < self.len() && self[i] != stop && i - start < max_len {
            i += 1;
        }
        if i > start && i <= self.len() {
            Some(&self[start..i])
        } else {
            None
        }
    }

    #[inline]
    fn hex_lower_into(&self, out: &mut [u8]) {
        hex_into(self, out, HEX_LOWER);
    }

    #[inline]
    fn b64_decode_into(&self, out: &mut [u8]) -> Result<usize, B64Error> {
        B64_STANDARD
            .decode_slice(self, out)
            .map_err(|_| B64Error::Malformed)
    }

    fn b64_field_decode(&self, out: &mut SmallVec<[u8; 4096]>, max_out: usize) -> Result<usize, B64Error> {
        let src = self.trim_ascii_extra(b"\"'");
        let need = b64_decode_cap(src.len());
        if need > max_out {
            return Err(B64Error::TooLarge(need));
        }
        out.clear();
        let req = B64_STANDARD.decoded_len_estimate(src.len());
        if req > max_out {
            return Err(B64Error::TooLarge(req));
        }
        out.reserve(req);
        unsafe {
            let ptr = out.as_mut_ptr();
            let dst = std::slice::from_raw_parts_mut(ptr, req);
            let n = src.b64_decode_into(dst)?;
            out.set_len(n);
            Ok(n)
        }
    }

    #[inline]
    fn b64_encode_into(&self, out: &mut [u8]) -> Result<usize, B64Error> {
        let cap = b64_encoded_len(self.len());
        if out.len() < cap {
            return Err(B64Error::TooLarge(cap));
        }
        B64_STANDARD
            .encode_slice(self, &mut out[..cap])
            .map_err(|_| B64Error::Malformed)
    }

    #[inline]
    fn b64_string(&self) -> String {
        B64_STANDARD.encode(self)
    }

    fn trim_ascii_extra(&self, extra: &[u8]) -> &Self {
        let is_junk = |b: u8| -> bool {
            matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) || extra.contains(&b)
        };
        let mut start = 0usize;
        let mut end = self.len();
        while start < end && is_junk(self[start]) {
            start += 1;
        }
        while end > start && is_junk(self[end - 1]) {
            end -= 1;
        }
        &self[start..end]
    }

    #[inline]
    fn eq_ci(&self, other: &[u8]) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    fn starts_with_ci(&self, prefix: &[u8]) -> bool {
        self.len() >= prefix.len() && self[..prefix.len()].eq_ci(prefix)
    }

    fn ends_with_ci(&self, suffix: &[u8]) -> bool {
        self.len() >= suffix.len() && self[self.len() - suffix.len()..].eq_ci(suffix)
    }

    fn contains_word(&self, word: &[u8]) -> bool {
        let Some(&first) = word.first() else {
            return true;
        };
        let mut start = 0usize;
        while start + word.len() <= self.len() {
            let rel = match memchr::memchr(first, &self[start..self.len() - word.len() + 1]) {
                Some(r) => r,
                None => break,
            };
            let hit = start + rel;
            let before_ok = hit == 0 || self[hit - 1] == b' ';
            let end = hit + word.len();
            let after_ok = end == self.len() || self[end] == b' ';
            if before_ok && after_ok && &self[hit..end] == word {
                return true;
            }
            start = hit + 1;
        }
        false
    }

    #[inline]
    fn strip_prefix_ci(&self, prefix: &[u8]) -> Option<&Self> {
        if self.starts_with_ci(prefix) {
            Some(&self[prefix.len()..])
        } else {
            None
        }
    }

    fn contains_ci(&self, needle: &[u8]) -> bool {
        let Some(&first) = needle.first() else {
            return true;
        };
        if self.len() < needle.len() {
            return false;
        }
        let lo = ascii_lower_byte(first);
        let up = ascii_upper_byte(first);
        let variants: [u8; 2] = if lo == up { [lo, lo] } else { [lo, up] };
        let limit = self.len() - needle.len();
        let mut start = 0usize;
        while start <= limit {
            let rel = memchr::memchr2(variants[0], variants[1], &self[start..=limit]);
            let Some(rel) = rel else { break };
            let hit = start + rel;
            if self[hit..hit + needle.len()].eq_ci(needle) {
                return true;
            }
            start = hit + 1;
        }
        false
    }

    fn leb128(&self, pos: &mut usize) -> Option<u64> {
        let b0 = *self.get(*pos)?;
        if b0 & 0x80 == 0 {
            *pos += 1;
            return Some(u64::from(b0));
        }
        let mut val: u64 = u64::from(b0 & 0x7f);
        let mut shift = 7u32;
        let mut p = *pos + 1;
        loop {
            let b = *self.get(p)?;
            p += 1;
            val |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                *pos = p;
                return Some(val);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
}

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn hex_into(src: &[u8], out: &mut [u8], table: &[u8; 16]) {
    debug_assert_eq!(out.len(), src.len() * 2);
    for (i, b) in src.iter().enumerate() {
        out[i * 2] = table[(b >> 4) as usize];
        out[i * 2 + 1] = table[(*b & 0x0F) as usize];
    }
}

#[inline]
pub fn b64_decode_cap(n: usize) -> usize {
    n.saturating_mul(3) / 4 + 4
}

#[inline]
pub fn b64_encoded_len(raw_len: usize) -> usize {
    raw_len.div_ceil(3) * 4
}


#[inline(always)]
fn hex_nib(b: u8, low: bool, table: &[u8; 16]) -> u8 {
    table[usize::from(if low { b & 0x0F } else { b >> 4 })]
}

#[inline]
pub fn ascii_lower_byte(b: u8) -> u8 {
    b + 0x20 * (b.is_ascii_uppercase() as u8)
}

#[inline]
pub fn ascii_upper_byte(b: u8) -> u8 {
    b - 0x20 * (b.is_ascii_lowercase() as u8)
}

#[inline]
pub fn push_ascii_case_into(out: &mut compact_str::CompactString, s: &str, upper: bool) {
    let n = s.len();
    out.reserve(n);
    unsafe {
        let spare = out.spare_capacity_mut();
        for (i, &b) in s.as_bytes().iter().enumerate() {
            let mask = if upper {
                0x20 * u8::from(b.is_ascii_lowercase())
            } else {
                0x20 * u8::from(b.is_ascii_uppercase())
            };
            spare[i].write(b ^ mask);
        }
        let len = out.len();
        out.set_len(len + n);
    }
}

pub fn ascii_upper_compact(s: &str) -> compact_str::CompactString {
    let mut out = compact_str::CompactString::with_capacity(s.len());
    push_ascii_case_into(&mut out, s, true);
    out
}

pub fn ascii_lower_compact(s: &str) -> compact_str::CompactString {
    let mut out = compact_str::CompactString::with_capacity(s.len());
    push_ascii_case_into(&mut out, s, false);
    out
}

#[inline]
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

pub fn percent_encode_into(src: &[u8], out: &mut smallvec::SmallVec<[u8; 128]>) {
    let escapes = src.iter().filter(|&&b| !is_unreserved(b)).count();
    let total = src.len() + escapes * 2;
    out.reserve(total);
    let mut w = out.len();
    unsafe {
        let ptr = out.as_mut_ptr();
        for &b in src {
            if is_unreserved(b) {
                *ptr.add(w) = b;
                w += 1;
            } else {
                *ptr.add(w) = b'%';
                *ptr.add(w + 1) = HEX_UPPER[usize::from(b >> 4)];
                *ptr.add(w + 2) = HEX_UPPER[usize::from(b & 0x0F)];
                w += 3;
            }
        }
        out.set_len(w);
    }
}

const HEX_NIBBLE: [u8; 256] = {
    let mut t = [255u8; 256];
    let mut i = b'0';
    while i <= b'9' {
        t[i as usize] = i - b'0';
        i += 1;
    }
    let mut lower = b'a';
    while lower <= b'f' {
        t[lower as usize] = lower - b'a' + 10;
        t[(lower - 32) as usize] = lower - b'a' + 10;
        lower += 1;
    }
    t
};

trait ByteSink {
    fn sink_push(&mut self, b: u8);
}

impl ByteSink for Vec<u8> {
    #[inline]
    fn sink_push(&mut self, b: u8) {
        self.push(b);
    }
}

impl<A: smallvec::Array<Item = u8>> ByteSink for smallvec::SmallVec<A> {
    #[inline]
    fn sink_push(&mut self, b: u8) {
        self.push(b);
    }
}

fn percent_decode_bytes<const PLUS_AS_SPACE: bool, S: ByteSink>(bytes: &[u8], out: &mut S) {
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if PLUS_AS_SPACE && b == b'+' {
            out.sink_push(b' ');
            i += 1;
            continue;
        }
        if b == b'%' && i + 2 < bytes.len() {
            let hi = HEX_NIBBLE[bytes[i + 1] as usize];
            let lo = HEX_NIBBLE[bytes[i + 2] as usize];
            if hi != 255 && lo != 255 {
                out.sink_push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.sink_push(b);
        i += 1;
    }
}

pub fn percent_decode(src: &str) -> Vec<u8> {
    let bytes = src.as_bytes();
    if !bytes.contains(&b'%') {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    percent_decode_bytes::<false, _>(bytes, &mut out);
    out
}

pub fn percent_decode_cow(src: &str) -> std::borrow::Cow<'_, [u8]> {
    if !src.as_bytes().contains(&b'%') {
        return std::borrow::Cow::Borrowed(src.as_bytes());
    }
    std::borrow::Cow::Owned(percent_decode(src))
}

#[inline]
pub fn hex_compact(bytes: &[u8], upper: bool) -> compact_str::CompactString {
    let n = bytes.len() * 2;
    let table: &[u8; 16] = if upper { &HEX_UPPER } else { &HEX_LOWER };
    let mut out = compact_str::CompactString::with_capacity(n);
    unsafe {
        let spare = out.spare_capacity_mut();
        let dst = spare.as_mut_ptr() as *mut u8;
        for (i, &b) in bytes.iter().enumerate() {
            *dst.add(i * 2) = table[usize::from(b >> 4)];
            *dst.add(i * 2 + 1) = table[usize::from(b & 0x0F)];
        }
        out.set_len(n);
    }
    out
}

pub fn hex_grouped(
    bytes: &[u8],
    upper: bool,
    sep: char,
    groups: &[usize],
) -> compact_str::CompactString {
    let table = if upper { HEX_UPPER } else { HEX_LOWER };
    let hex_len = bytes.len() * 2;
    let mut seps = 0usize;
    let mut covered = 0usize;
    for &g in groups {
        if g == 0 || covered >= hex_len {
            break;
        }
        if covered > 0 {
            seps += 1;
        }
        covered = (covered + g).min(hex_len);
    }
    let mut sep_buf = [0u8; 4];
    let sep_str = sep.encode_utf8(&mut sep_buf);
    let sep_len = sep_str.len();
    let n = hex_len + seps * sep_len;
    let mut out = compact_str::CompactString::with_capacity(n);
    unsafe {
        let spare = out.spare_capacity_mut();
        let dst = spare.as_mut_ptr() as *mut u8;
        let mut w = 0usize;
        let mut rest = 0usize;
        let mut first = true;
        for &g in groups {
            if g == 0 || rest >= hex_len {
                break;
            }
            if !first {
                for k in 0..sep_len {
                    *dst.add(w + k) = sep_str.as_bytes()[k];
                }
                w += sep_len;
            }
            first = false;
            let take = g.min(hex_len - rest);
            for c in rest..rest + take {
                let b = bytes[c >> 1];
                *dst.add(w) = hex_nib(b, c & 1 == 1, table);
                w += 1;
            }
            rest += take;
        }
        for c in rest..hex_len {
            let b = bytes[c >> 1];
            *dst.add(w) = hex_nib(b, c & 1 == 1, table);
            w += 1;
        }
        out.set_len(w);
    }
    out
}

pub fn percent_encode_compact(src: &str) -> compact_str::CompactString {
    let mut buf: smallvec::SmallVec<[u8; 128]> = smallvec::SmallVec::new();
    percent_encode_into(src.as_bytes(), &mut buf);
    compact_str::CompactString::new(unsafe { std::str::from_utf8_unchecked(buf.as_slice()) })
}

pub fn form_urlencoded_encode(src: &str) -> CompactString {
    let mut out = CompactString::with_capacity(src.len());
    for &b in src.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'*' | b'-' | b'.' | b'_') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(HEX_UPPER[(b >> 4) as usize] as char);
            out.push(HEX_UPPER[(b & 0x0F) as usize] as char);
        }
    }
    out
}

pub fn form_urlencoded_decode(src: &str) -> CompactString {
    if !src.bytes().any(|b| b == b'+' || b == b'%') {
        return CompactString::from(src);
    }
    let mut buf: SmallVec<[u8; 64]> = SmallVec::new();
    percent_decode_bytes::<true, _>(src.as_bytes(), &mut buf);
    CompactString::from_utf8_lossy(buf.as_slice())
}
