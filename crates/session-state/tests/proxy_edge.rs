use session_state::{
    ProxyConfig, ProxyParseError, ProxyScheme, assess_proxy, check_tz_ip_consistency,
    chrome_full_version, classify_asn, geo_for_host, grease_brand_set, is_safe_for_signup,
};

#[test]
fn proxy_parse_rejects_empty_and_schemeless() {
    assert!(matches!(
        ProxyConfig::parse(""),
        Err(ProxyParseError::Empty)
    ));
    assert!(matches!(
        ProxyConfig::parse("   "),
        Err(ProxyParseError::Empty)
    ));
    assert!(matches!(
        ProxyConfig::parse("host.example:8080"),
        Err(ProxyParseError::MissingScheme)
    ));
    assert!(matches!(
        ProxyConfig::parse("ftp://host.example:21"),
        Err(ProxyParseError::UnsupportedScheme)
    ));
}

#[test]
fn proxy_parse_rejects_missing_host_and_port() {
    assert!(matches!(
        ProxyConfig::parse("http://"),
        Err(ProxyParseError::MissingHost)
    ));
    assert!(matches!(
        ProxyConfig::parse("socks5://host"),
        Err(ProxyParseError::MissingPort)
    ));
    assert!(matches!(
        ProxyConfig::parse("http://host"),
        Ok(cfg) if cfg.port == 80
    ));
    assert!(matches!(
        ProxyConfig::parse("http://:80"),
        Err(ProxyParseError::MissingHost)
    ));
    assert!(matches!(
        ProxyConfig::parse("http://host:0"),
        Err(ProxyParseError::BadPort)
    ));
    assert!(matches!(
        ProxyConfig::parse("http://host:70000"),
        Err(ProxyParseError::BadPort)
    ));
    assert!(matches!(
        ProxyConfig::parse("http://host:abc"),
        Err(ProxyParseError::BadPort)
    ));
}

#[test]
fn proxy_parse_plain_and_auth() {
    let p = ProxyConfig::parse("socks5://1.2.3.4:1080").expect("parse ok");
    assert_eq!(p.scheme, ProxyScheme::Socks5);
    assert_eq!(p.host().as_str(), "1.2.3.4");
    assert_eq!(p.port, 1080);
    assert!(p.auth.is_none());
    assert!(p.wreq_compatible());
    assert_eq!(p.to_url_string().as_str(), "socks5://1.2.3.4:1080");

    let p = ProxyConfig::parse("http://user%40x:p%3Ass@proxy.example.com:3128").expect("auth ok");
    let a = p.auth.clone().expect("auth present");
    assert_eq!(a.username.as_str(), "user@x");
    assert_eq!(a.password.as_str(), "p:ss");
    let url = p.to_url_string();
    assert!(url.starts_with("http://user%40x:p%3Ass@proxy.example.com:3128"));
}

#[test]
fn proxy_parse_utc_query() {
    let p = ProxyConfig::parse("http://host.example:8080?utc=60").expect("utc ok");
    assert_eq!(p.utc_offset, Some(60));
    let p = ProxyConfig::parse("http://host.example:8080?utc=-300&x=1").expect("utc neg");
    assert_eq!(p.utc_offset, Some(-300));
    let p = ProxyConfig::parse("http://host.example:8080").expect("no utc");
    assert_eq!(p.utc_offset, None);
}

#[test]
fn proxy_parse_huge_url_rejected() {
    let long_host = "a".repeat(600);
    let url = format!("http://{long_host}:80");

    assert!(matches!(
        ProxyConfig::parse(&url),
        Err(ProxyParseError::TooLong)
    ));
}

#[test]
fn proxy_parse_utc_query_boundary() {
    let p = ProxyConfig::parse("http://host.example:8080?flautc=999").expect("parse ok");
    assert_eq!(p.utc_offset, None, "flautc= не должен матчиться как utc=");
    let p = ProxyConfig::parse("http://host.example:8080?x=1&utc=90").expect("utc после &");
    assert_eq!(p.utc_offset, Some(90));
}

#[test]
fn proxy_ipv6_roundtrip() {
    let p = ProxyConfig::parse("http://[2001:db8::1]:3128").expect("v6 ok");
    assert_eq!(p.host().as_str(), "2001:db8::1");
    assert_eq!(p.port, 3128);

    assert_eq!(p.to_url_string().as_str(), "http://[2001:db8::1]:3128");
    let p = ProxyConfig::parse("socks5://[::1]:1080").expect("v6 socks");
    assert_eq!(p.to_url_string().as_str(), "socks5://[::1]:1080");
}

#[test]
fn proxy_ipv6_with_userinfo() {
    let p = ProxyConfig::parse("http://user:pass@[2001:db8::1]:3128").expect("v6+auth ok");
    assert_eq!(p.host().as_str(), "2001:db8::1");
    assert_eq!(p.port, 3128);
    let a = p.auth.as_ref().expect("auth present");
    assert_eq!(a.username.as_str(), "user");
    assert_eq!(a.password.as_str(), "pass");
    assert_eq!(
        p.to_url_string().as_str(),
        "http://user:pass@[2001:db8::1]:3128"
    );
}

#[test]
fn grease_brands_are_consistent_for_every_known_major() {
    for major in 131..=149u32 {
        let brands = grease_brand_set(major);
        let has_chromium = brands.iter().any(|(b, _)| *b == "Chromium");
        let has_chrome = brands.iter().any(|(b, _)| *b == "Google Chrome");
        assert!(
            has_chromium && has_chrome,
            "major {major} lost a real brand"
        );
        for (brand, version) in brands {
            if brand == "Google Chrome" || brand == "Chromium" {
                assert_eq!(
                    version.parse::<u32>().expect("numeric version"),
                    major,
                    "major {major} brand version mismatch"
                );
            }
        }
        assert!(
            chrome_full_version(major).is_some(),
            "full version for {major}"
        );
    }
}

#[test]
fn chrome_full_version_rejects_unknown() {
    assert!(chrome_full_version(130).is_none());
    assert!(chrome_full_version(200).is_none());
    assert_eq!(chrome_full_version(147), Some("147.0.7712.122"));
}

#[test]
fn chrome_full_version_branches_are_monotonic() {
    let mut prev_major = 0u32;
    let mut prev_branch = 0u32;
    for major in 131..=149u32 {
        let full = chrome_full_version(major).expect("version present");
        let mut it = full.split('.');
        assert_eq!(
            it.next().unwrap().parse::<u32>().unwrap(),
            major,
            "major {major}: first component must equal major in {full}"
        );
        let branch: u32 = it
            .nth(1)
            .expect("branch component")
            .parse()
            .expect("numeric branch");
        assert!(
            major > prev_major && branch > prev_branch,
            "branch regression: major {major} branch {branch} <= previous major {prev_major} branch {prev_branch} ({full})"
        );
        prev_major = major;
        prev_branch = branch;
    }
}

#[test]
fn asn_classification() {
    assert_eq!(classify_asn(15169).as_str(), "datacenter");
    assert_eq!(
        classify_asn(24940).as_str(),
        "datacenter",
        "hetzner sits in both lists, datacenter wins per source order"
    );
    assert_eq!(
        classify_asn(9009).as_str(),
        "datacenter",
        "m247 is datacenter in the carrier registry, the vpn list overlaps"
    );
    assert_eq!(classify_asn(197540).as_str(), "vpn");
    assert_eq!(classify_asn(197727).as_str(), "tor");
    assert_eq!(classify_asn(3320).as_str(), "mobile");
    assert_eq!(classify_asn(12345).as_str(), "residential");
}

#[test]
fn geo_for_host_hits_and_misses() {
    let g = geo_for_host("proxy.mullvad.de").expect("de geo");
    assert_eq!(g.tz, "Europe/Berlin");

    assert_eq!(core_utils::tz_offset_for(g.tz, 1_768_478_400_000), 60);
    let g = geo_for_host("gate.example.com").expect("com geo");
    assert_eq!(g.tz, "America/New_York");
    assert!(geo_for_host("localhost").is_none());
    assert!(geo_for_host("").is_none());
}

#[test]
fn tz_ip_consistency_detects_mismatch() {
    let expected = core_utils::tz_offset_for("Europe/Berlin", core_utils::unix_ms() as i64);
    assert!(check_tz_ip_consistency("proxy.example.de", expected).is_none());
    let mismatch = check_tz_ip_consistency("proxy.example.de", 540);
    assert!(mismatch.is_some());
    assert_eq!(mismatch.unwrap(), (expected, 540));
    assert!(check_tz_ip_consistency("unknown.local", 0).is_none());
}

#[test]
fn every_referenced_zone_exists_in_tz_db() {
    for row in session_state::ASN_REG {
        if row.tz.is_empty() {
            continue;
        }
        assert!(
            core_utils::zone_of(row.tz).is_some(),
            "ASN_REG zone {:?} missing in core_utils::ZONES",
            row.tz
        );
    }
    for (_, g) in session_state::GEO_TABLE.iter() {
        assert!(
            core_utils::zone_of(g.tz).is_some(),
            "GEO_TABLE zone {:?} missing in core_utils::ZONES",
            g.tz
        );
    }
}

#[test]
fn assess_proxy_scores() {
    let a = assess_proxy(12345, "US", "America/New_York", "en-US");
    assert!(a.score > 0.9);
    assert!(is_safe_for_signup(&a));

    let dc = assess_proxy(15169, "US", "America/New_York", "en-US");
    assert!(dc.score < 0.6);
    assert!(!is_safe_for_signup(&dc));

    let tz_bad = assess_proxy(12345, "DE", "Asia/Tokyo", "en-US");
    assert!(!is_safe_for_signup(&tz_bad));
}
