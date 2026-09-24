use net_client::{
    EngineSet, HDR_AKAMAI, HDR_CF_MITIGATED_CHALLENGE, HDR_CF_RAY, HDR_DD_B, HDR_KPSDK, HDR_PX,
    VENDOR_AKAMAI, VENDOR_CLOUDFLARE, VENDOR_DATADOME, VENDOR_GENERIC, VENDOR_KASADA, VENDOR_NONE,
    VENDOR_PERIMETERX, challenge_vendor_of, engine_catalog, fetch_page, guard_url, is_forbidden_ip,
    reslot_with_asn, vendor_label,
};
use session_state::Session;

#[test]
fn guard_rejects_empty_and_schemeless() {
    assert!(guard_url("").is_err());
    assert!(guard_url("not a url").is_err());
    assert!(guard_url("example.com/path").is_err());
}

#[test]
fn guard_rejects_non_http_schemes() {
    assert!(guard_url("ftp://example.com/file").is_err());
    assert!(guard_url("file:///etc/passwd").is_err());
    assert!(guard_url("javascript:alert(1)").is_err());
    assert!(guard_url("data:text/plain,hi").is_err());
}

#[test]
fn guard_blocks_localhost_and_private() {
    for url in [
        "http://localhost/x",
        "http://localhost:8080/x",
        "http://sub.localhost/x",
        "http://127.0.0.1/x",
        "http://10.0.0.1/x",
        "http://192.168.1.1/admin",
        "http://172.16.0.5/x",
        "http://169.254.169.254/latest/meta-data",
        "http://0.0.0.0/x",
        "http://[::1]/x",
        "http://[fe80::1]/x",
        "http://[fd00::1]/x",
        "http://[::ffff:127.0.0.1]/x",
    ] {
        assert!(guard_url(url).is_err(), "must block {url}");
    }
}

#[test]
fn guard_allows_public() {
    for url in [
        "https://example.com/",
        "http://93.184.216.34/",
        "https://sub.site.org/deep/path?q=1",
    ] {
        assert!(guard_url(url).is_ok(), "must allow {url}");
    }
}

#[test]
fn forbidden_ip_vectors() {
    use std::net::IpAddr;
    let bad = [
        "127.0.0.1",
        "10.1.2.3",
        "192.168.0.1",
        "172.31.255.255",
        "169.254.1.1",
        "0.0.0.0",
        "255.255.255.255",
        "::1",
        "::",
        "fe80::1",
        "fd12::1",
        "::ffff:10.0.0.1",
    ];
    for ip in bad {
        let ip: IpAddr = ip.parse().expect("ip parse");
        assert!(is_forbidden_ip(ip), "must be forbidden: {ip}");
    }
    let good = ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"];
    for ip in good {
        let ip: IpAddr = ip.parse().expect("ip parse");
        assert!(!is_forbidden_ip(ip), "must be allowed: {ip}");
    }
}

#[test]
fn vendor_detection_from_flags_and_body() {
    assert_eq!(
        challenge_vendor_of(403, HDR_CF_MITIGATED_CHALLENGE, b""),
        VENDOR_CLOUDFLARE
    );
    assert_eq!(
        challenge_vendor_of(503, HDR_CF_RAY, b"Just a moment..."),
        VENDOR_CLOUDFLARE
    );
    assert_eq!(
        challenge_vendor_of(403, 0, b"<!doctype html>cf-challenge"),
        VENDOR_CLOUDFLARE
    );
    assert_eq!(challenge_vendor_of(200, HDR_DD_B, b""), VENDOR_DATADOME);
    assert_eq!(
        challenge_vendor_of(403, 0, b"<html>datadome</html>"),
        VENDOR_DATADOME
    );
    assert_eq!(challenge_vendor_of(200, HDR_KPSDK, b""), VENDOR_KASADA);
    assert_eq!(challenge_vendor_of(200, HDR_PX, b""), VENDOR_PERIMETERX);
    assert_eq!(
        challenge_vendor_of(200, HDR_AKAMAI, b"<script src=/_sec/x></script>"),
        VENDOR_AKAMAI
    );
    assert_eq!(challenge_vendor_of(429, 0, b""), VENDOR_GENERIC);
    assert_eq!(challenge_vendor_of(403, 0, b"plain denial"), VENDOR_GENERIC);
    assert_eq!(challenge_vendor_of(200, 0, b"ok"), VENDOR_NONE);
}

#[test]
fn vendor_label_roundtrip() {
    for v in [
        VENDOR_NONE,
        VENDOR_CLOUDFLARE,
        VENDOR_DATADOME,
        VENDOR_KASADA,
        VENDOR_PERIMETERX,
        VENDOR_AKAMAI,
        VENDOR_GENERIC,
    ] {
        let label = vendor_label(v);
        assert!(!label.is_empty());
    }
    assert_eq!(vendor_label(200), "none");
}

#[test]
fn guard_blocks_integer_and_hex_ip_forms() {
    for url in [
        "http://2130706433/x",
        "http://0x7f000001/x",
        "http://0177.0.0.1/x",
        "http://0/x",
        "http://0.0.0.0/x",
        "http://0x7f.0.0.1/x",
        "http://127.1/x",
    ] {
        assert!(guard_url(url).is_err(), "must fold and block {url}");
    }
}

#[test]
fn guard_blocks_userinfo_bypass() {
    for url in [
        "http://evil.com@127.0.0.1/x",
        "http://innocent.example@169.254.169.254/latest",
        "http://a:b@10.0.0.1/x",
    ] {
        assert!(guard_url(url).is_err(), "host is the tail after '@': {url}");
    }
    assert!(guard_url("http://user:pass@example.com/x").is_ok());
}

#[test]
fn guard_scheme_is_case_insensitive() {
    assert!(guard_url("HTTP://example.com/x").is_ok());
    assert!(guard_url("Https://example.com/x").is_ok());
    assert!(guard_url("FTP://example.com/x").is_err());
}

#[test]
fn guard_passes_public_hosts_and_ipv6_literals() {
    for url in [
        "https://example.com/",
        "http://sub.example.com:8443/path?q=1",
        "http://[2606:4700::6810:85e5]/x",
        "http://8.8.8.8/x",
        "http://1.1.1.1/x",
    ] {
        assert!(guard_url(url).is_ok(), "must allow {url}");
    }
}

#[test]
fn guard_blocks_percent_encoded_and_fullwidth_ip_bypasses() {
    assert!(guard_url("http://%31%32%37.0.0.1/").is_err());
    assert!(guard_url("http://127%2e0%2e0%2e1/").is_err());

    assert!(guard_url("http://127。0。0。1/").is_err());
    assert!(guard_url("http://127．0．0．1/").is_err());
    assert!(guard_url("http://127｡0｡0｡1/").is_err());

    assert!(guard_url("http://%30%78%37%66.0.0.1/").is_err());

    assert!(guard_url("http://%4cOCALHOST/").is_err());
    assert!(guard_url("http://LOCAL%48OST/").is_err());

    assert!(guard_url("http://１２７.0.0.1/").is_err());
    assert!(guard_url("http://１２７．０．０．１/").is_err());
    assert!(guard_url("http://ＬＯＣＡＬＨＯＳＴ/").is_err());

    assert!(guard_url("http://127.0.0.1\n/").is_err());
    assert!(guard_url("http://127.0.0.1\t/").is_err());
    assert!(guard_url("http://localhost\r/").is_err());
}

#[tokio::test]
async fn malformed_url_fails_gracefully() {
    let catalog = engine_catalog().expect("catalog");
    let engines = EngineSet::build(&catalog).expect("engines");
    let profile = reslot_with_asn(catalog[0].profile.as_ref(), 0);
    let mut session = Session::new(profile, "about:blank");
    let r = fetch_page(&engines, 0, &mut session, "htp:/::bad-url").await;
    assert!(r.is_err());
}

#[tokio::test]
async fn private_target_fetch_fails_gracefully() {
    let catalog = engine_catalog().expect("catalog");
    let engines = EngineSet::build(&catalog).expect("engines");
    let profile = reslot_with_asn(catalog[0].profile.as_ref(), 0);
    let mut session = Session::new(profile, "http://127.0.0.1:9/");
    let r = fetch_page(&engines, 0, &mut session, "http://127.0.0.1:9/").await;
    let err = r.err().expect("must fail");
    assert!(
        err.to_string().contains("transport")
            || err.to_string().contains("os error 111")
            || err.to_string().contains("blocked"),
        "unexpected: {err}"
    );
}
