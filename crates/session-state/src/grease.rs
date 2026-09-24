use compact_str::CompactString;

const GREASE_CHARS: [&str; 11] = [
    " ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_",
];

const GREASE_COMBOS: usize = 11 * 11;

const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];

#[rustfmt::skip]
const SHUFFLE_ORDERS: [[usize; 3]; 6] = [[0,1,2],[0,2,1],[1,0,2],[1,2,0],[2,0,1],[2,1,0]];

pub type BrandPair = (&'static str, &'static str);

pub const WREQ_CHROME_BRANDS: &[(u32, [BrandPair; 3])] = &[
    (131, [("Google Chrome", "131"), ("Chromium", "131"), ("Not_A Brand", "24")]),
    (132, [("Not A(Brand", "8"), ("Chromium", "132"), ("Google Chrome", "132")]),
    (133, [("Not(A:Brand", "99"), ("Google Chrome", "133"), ("Chromium", "133")]),
    (134, [("Chromium", "134"), ("Not:A-Brand", "24"), ("Google Chrome", "134")]),
    (135, [("Google Chrome", "135"), ("Not-A.Brand", "8"), ("Chromium", "135")]),
    (136, [("Chromium", "136"), ("Google Chrome", "136"), ("Not.A/Brand", "99")]),
    (137, [("Google Chrome", "137"), ("Chromium", "137"), ("Not/A)Brand", "24")]),
    (138, [("Not)A;Brand", "8"), ("Chromium", "138"), ("Google Chrome", "138")]),
    (139, [("Not;A=Brand", "99"), ("Google Chrome", "139"), ("Chromium", "139")]),
    (140, [("Chromium", "140"), ("Not=A?Brand", "24"), ("Google Chrome", "140")]),
    (141, [("Google Chrome", "141"), ("Not?A_Brand", "8"), ("Chromium", "141")]),
    (142, [("Chromium", "142"), ("Google Chrome", "142"), ("Not_A Space", "99")]),
    (143, [("Google Chrome", "143"), ("Chromium", "143"), ("Not A(Brand", "24")]),
    (144, [("Not(A:Brand", "8"), ("Chromium", "144"), ("Google Chrome", "144")]),
    (145, [("Not:A-Brand", "99"), ("Google Chrome", "145"), ("Chromium", "145")]),
    (146, [("Chromium", "146"), ("Not-A.Brand", "24"), ("Google Chrome", "146")]),
    (147, [("Google Chrome", "147"), ("Chromium", "147"), ("Not.A/Brand", "8")]),
    (148, [("Chromium", "148"), ("Google Chrome", "148"), ("Not/A)Brand", "99")]),
    (149, [("Google Chrome", "149"), ("Chromium", "149"), ("Not_A Brand", "24")]),
];

pub const CHROME_FULL_VERSIONS: &[(u32, &str)] = &[
    (131, "131.0.6778.205"),
    (132, "132.0.6834.159"),
    (133, "133.0.6943.127"),
    (134, "134.0.6998.166"),
    (135, "135.0.7049.95"),
    (136, "136.0.7103.114"),
    (137, "137.0.7151.69"),
    (138, "138.0.7204.157"),
    (139, "139.0.7258.66"),
    (140, "140.0.7339.81"),
    (141, "141.0.7390.54"),
    (142, "142.0.7445.94"),
    (143, "143.0.7544.97"),
    (144, "144.0.7554.104"),
    (145, "145.0.7565.59"),
    (146, "146.0.7680.164"),
    (147, "147.0.7712.122"),
    (148, "148.0.7738.0"),
    (149, "149.0.7770.0"),
];

#[inline]
pub fn chrome_full_version(major: u32) -> Option<&'static str> {
    CHROME_FULL_VERSIONS
        .binary_search_by(|(m, _)| m.cmp(&major))
        .ok()
        .map(|i| CHROME_FULL_VERSIONS[i].1)
}

#[inline]
fn canonical_wreq_brands(major: u32) -> Option<[BrandPair; 3]> {
    WREQ_CHROME_BRANDS
        .binary_search_by(|(m, _)| m.cmp(&major))
        .ok()
        .map(|i| WREQ_CHROME_BRANDS[i].1)
}

const fn combo_bytes() -> [[u8; 11]; GREASE_COMBOS] {
    let mut t = [[0u8; 11]; GREASE_COMBOS];
    let mut i = 0usize;
    while i < GREASE_COMBOS {
        let a = GREASE_CHARS[i / GREASE_CHARS.len()].as_bytes()[0];
        let b = GREASE_CHARS[i % GREASE_CHARS.len()].as_bytes()[0];
        t[i] = [b'N', b'o', b't', a, b'A', b, b'B', b'r', b'a', b'n', b'd'];
        i += 1;
    }
    t
}

static COMBOS: [[u8; 11]; GREASE_COMBOS] = combo_bytes();

#[inline]
fn combo_name(i: usize) -> &'static str {
    unsafe { core::str::from_utf8_unchecked(&COMBOS[i]) }
}

const fn major_bytes() -> [[u8; 3]; 41] {
    let mut t = [[0u8; 3]; 41];
    let mut m = 120u32;
    while m <= 160 {
        let mut buf = [0u8; 20];
        core_utils::u64_digits_fixed_into(m as u64, &mut buf, 3);
        t[(m - 120) as usize] = [buf[0], buf[1], buf[2]];
        m += 1;
    }
    t
}

static MAJORS: [[u8; 3]; 41] = major_bytes();

#[inline]
fn major_name(i: usize) -> &'static str {
    unsafe { core::str::from_utf8_unchecked(&MAJORS[i]) }
}

pub fn grease_brand_set(major: u32) -> [BrandPair; 3] {
    if let Some(canonical) = canonical_wreq_brands(major) {
        return canonical;
    }
    let seed = major as usize;
    let greased =
        combo_name(seed % GREASE_CHARS.len() * GREASE_CHARS.len() + (seed + 1) % GREASE_CHARS.len());
    let major_str = match major {
        120..=160 => major_name((major - 120) as usize),
        _ => "0",
    };
    let source: [BrandPair; 3] = [
        (greased, GREASE_VERSIONS[seed % GREASE_VERSIONS.len()]),
        ("Chromium", major_str),
        ("Google Chrome", major_str),
    ];
    let order = SHUFFLE_ORDERS[seed % SHUFFLE_ORDERS.len()];
    let mut out = source;
    let mut i = 0usize;
    while i < 3 {
        out[order[i]] = source[i];
        i += 1;
    }
    out
}

pub fn sec_chua_header_of(brands: &[BrandPair; 3]) -> CompactString {
    let mut out = CompactString::with_capacity(96);
    for (i, (brand, version)) in brands.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(brand);
        out.push_str("\";v=\"");
        out.push_str(version);
        out.push('"');
    }
    out
}
