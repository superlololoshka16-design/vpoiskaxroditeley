use core_utils::*;

#[test]
fn hex_compact_roundtrip_case() {
    assert_eq!(
        hex_compact(&[0xDE, 0xAD, 0xBE, 0xEF], false).as_str(),
        "deadbeef"
    );
    assert_eq!(hex_compact(&[0xde, 0xad], true).as_str(), "DEAD");
    assert_eq!(hex_compact(&[], false).as_str(), "");
}

#[test]
fn hex_grouped_uuid_and_mdns_shapes() {
    let b = [
        0x12u8, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88,
    ];
    assert_eq!(
        hex_grouped(&b, false, '-', &[8, 4, 4, 4, 12]).as_str(),
        "12345678-9abc-def0-1122-334455667788"
    );
    let m = [0x01u8, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
    assert_eq!(
        hex_grouped(&m, false, '-', &[8, 4, 4]).as_str(),
        "01234567-89ab-cdef"
    );
    let d = [0xABu8, 0xCD];
    assert_eq!(hex_grouped(&d, true, ':', &[2, 2]).as_str(), "AB:CD");
    assert_eq!(hex_grouped(&d, false, '-', &[]).as_str(), "abcd");
}

#[test]
fn percent_roundtrip() {
    let enc = percent_encode_compact("a b/c?d#e");
    assert_eq!(enc.as_str(), "a%20b%2Fc%3Fd%23e");
    assert_eq!(percent_decode(enc.as_str()), b"a b/c?d#e".to_vec());
    assert_eq!(percent_decode("100%"), b"100%".to_vec());
    assert_eq!(percent_decode("%zz"), b"%zz".to_vec());
}

#[test]
fn b64_rejects_and_decodes_hostile() {
    use core_utils::BytesExt as _;
    let mut out = [0u8; 16];
    assert!(b"SGVsbG8gd29ybGQh".b64_decode_into(&mut out).is_ok());
    assert!(b"!!!!".b64_decode_into(&mut out).is_err());
    assert!(b"".b64_decode_into(&mut out).is_ok());
    assert!(b"YWJj".b64_decode_into(&mut [0u8; 2]).is_err());
}

#[test]
fn percent_encode_compact_no_forced_heap() {
    let s = percent_encode_compact("a b&c");
    assert_eq!(s.as_str(), "a%20b%26c");

    let s = percent_encode_compact("ю");
    assert_eq!(s.as_str(), "%D1%8E");
}

#[test]
fn ascii_lower_compact_ascii_only_and_utf8_safe() {
    assert_eq!(ascii_lower_compact("ExAmPlE.COM").as_str(), "example.com");
    assert_eq!(ascii_lower_compact("").as_str(), "");
    let idn = "Exämple.рф";
    let lowered = ascii_lower_compact(idn);
    assert_eq!(lowered.as_str(), "exämple.рф");
    assert_eq!(lowered.len(), idn.len());
    assert_eq!(ascii_lower_compact("ÜÜ").as_str(), "ÜÜ");
}
