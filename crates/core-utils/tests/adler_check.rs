#[test]
fn adler32_matches_zlib_reference() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let got = core_utils::adler32_feed(1, &data);
    assert_eq!(got, 2942630, "adler32 не совпал с эталоном zlib");
    let empty = core_utils::adler32_feed(1, &[]);
    assert_eq!(empty, 1);
    let zeros = core_utils::adler32_feed(1, &[0u8; 6000]);
    fn ref_adler(d: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in d { a = (a + x as u32) % 65521; b = (b + a) % 65521; }
        (b << 16) | a
    }
    assert_eq!(zeros, ref_adler(&[0u8; 6000]), "adler32_zeros разошёлся на >NMAX");
    let big: Vec<u8> = (0..20000u32).map(|i| (i % 256) as u8).collect();
    assert_eq!(core_utils::adler32_feed(1, &big), ref_adler(&big), "большой блок");
}
