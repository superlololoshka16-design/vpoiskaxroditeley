use net_client::charset::{
    charset_from_content_type, encoding_for_label, is_utf8, sniff_meta_charset,
};

#[test]
fn charset_from_content_type_variants() {
    assert_eq!(
        charset_from_content_type("text/html; charset=UTF-8"),
        Some("UTF-8")
    );
    assert_eq!(
        charset_from_content_type("text/html;charset=Shift_JIS"),
        Some("Shift_JIS")
    );
    assert_eq!(
        charset_from_content_type("text/html; charset=\"windows-1252\""),
        Some("windows-1252")
    );
    assert_eq!(charset_from_content_type("text/html"), None);
    assert_eq!(charset_from_content_type(""), None);
}

#[test]
fn meta_sniff_finds_charset() {
    let html = b"<html><head><meta charset=\"windows-1252\"><title>x</title>";
    assert_eq!(sniff_meta_charset(html), Some("windows-1252"));
    let legacy = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=Shift_JIS\">";
    assert_eq!(sniff_meta_charset(legacy), Some("Shift_JIS"));
    assert!(sniff_meta_charset(b"<html><body>no meta</body>").is_none());
    assert!(sniff_meta_charset(b"").is_none());

    assert_eq!(
        sniff_meta_charset(b"<meta charset=\"windows-1251\">"),
        Some("windows-1251")
    );
    let enc = encoding_for_label(sniff_meta_charset(b"<meta charset=\"windows-1251\">").unwrap());
    assert_eq!(enc, Some(encoding_rs::WINDOWS_1251));
}

#[test]
fn garbage_head_does_not_panic() {
    let garbage: Vec<u8> = (0..1024u32).map(|i| (i % 256) as u8).collect();
    let _ = sniff_meta_charset(&garbage);
    let cut = b"<meta char";
    let _ = sniff_meta_charset(cut);
}

#[test]
fn labels_full_whatwg_coverage() {
    assert!(encoding_for_label("windows-1251").is_some());
    assert!(encoding_for_label("Shift_JIS").is_some());
    assert!(encoding_for_label("GBK").is_some());
    assert!(encoding_for_label("EUC-JP").is_some());
    assert!(encoding_for_label("koi8-r").is_some());
    assert!(encoding_for_label("iso-8859-15").is_some());

    assert_eq!(
        encoding_for_label("iso-8859-1"),
        Some(encoding_rs::WINDOWS_1252)
    );
    assert!(is_utf8(encoding_for_label("utf-8").unwrap()));
    assert!(!is_utf8(encoding_for_label("koi8-r").unwrap()));
    assert!(encoding_for_label("not-a-charset").is_none());
}

#[test]
fn decode_streaming_cp1251_and_shiftjis() {
    let cp1251 = encoding_rs::WINDOWS_1251;
    let src: &[u8] = &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];
    let mut dec = cp1251.new_decoder();
    let mut out = String::with_capacity(16);
    for b in src {
        let (rv, _, _) = dec.decode_to_string(&[*b], &mut out, false);
        assert!(matches!(rv, encoding_rs::CoderResult::InputEmpty));
    }
    assert_eq!(out, "Привет");

    let sjis = encoding_rs::SHIFT_JIS;
    let mut dec = sjis.new_decoder();
    let mut out = String::with_capacity(8);
    let (rv, _, _) = dec.decode_to_string(&[0x93], &mut out, false);
    assert!(matches!(rv, encoding_rs::CoderResult::InputEmpty));
    assert!(
        out.is_empty(),
        "незакрытая последовательность не декодится досрочно"
    );
    let (rv, _, _) = dec.decode_to_string(&[0xFA], &mut out, false);
    assert!(matches!(rv, encoding_rs::CoderResult::InputEmpty));
    assert_eq!(out, "日");
}

#[test]
fn decode_windows1252_euro() {
    let enc = encoding_rs::WINDOWS_1252;
    let (s, _, _) = enc.decode(&[0x80, 0x41]);
    assert_eq!(s, "\u{20AC}A");

    let (s, _, _) = encoding_rs::WINDOWS_1252.decode(&[0x41, 0xE9]);
    assert_eq!(s, "A\u{E9}");
}
