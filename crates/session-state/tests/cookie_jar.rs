use session_state::CookieJar;

#[test]
fn flat_ingest_keeps_legacy_surface() {
    let mut jar = CookieJar::new();
    jar.ingest("session=abc123; Path=/; HttpOnly");
    jar.ingest("csrf=tok1");
    assert_eq!(jar.get("session"), Some("abc123"));
    assert_eq!(jar.get("csrf"), Some("tok1"));
    assert_eq!(jar.len(), 2);
    let header = jar.header_str().expect("header");
    assert!(header.contains("session=abc123"));
    assert!(header.contains("csrf=tok1"));
}

#[test]
fn ingest_rejects_garbage() {
    let mut jar = CookieJar::new();
    jar.ingest("");
    jar.ingest("novalue");
    jar.ingest("=value");
    jar.ingest("   =   ");
    jar.ingest(&format!("{}=x", "a".repeat(300)));
    jar.ingest(&format!("k={}", "v".repeat(5000)));
    assert!(jar.is_empty());
}

#[test]
fn domain_scoped_ingest_and_match() {
    let mut jar = CookieJar::new();
    jar.ingest_for_url(
        "SID=aaa; Domain=.example.com; Path=/",
        "https://www.example.com/page",
    );
    assert_eq!(jar.get_for_host("SID", "other.example.com"), Some("aaa"));
    assert_eq!(jar.get_for_host("SID", "www.example.com"), Some("aaa"));
    assert_eq!(jar.get_for_host("SID", "notexample.com"), None);
    let header = jar
        .header_for_url("https://api.example.com/v1/list")
        .expect("cookie visible");
    assert!(header.contains("SID=aaa"));
}

#[test]
fn host_only_cookie_never_leaks_to_subdomain() {
    let mut jar = CookieJar::new();
    jar.ingest_for_url("hostonly=1", "https://a.example.com/x");
    assert_eq!(jar.get_for_host("hostonly", "a.example.com"), Some("1"));
    assert_eq!(jar.get_for_host("hostonly", "b.example.com"), None);
}

#[test]
fn domain_attr_from_foreign_host_ignored() {
    let mut jar = CookieJar::new();
    jar.ingest_for_url("evil=1; Domain=other.com", "https://example.com/page");
    assert_eq!(
        jar.get_for_host("evil", "sub.other.com"),
        None,
        "cookie for foreign domain must not be planted"
    );
}

#[test]
fn path_scoped_match() {
    let mut jar = CookieJar::new();
    jar.ingest_for_url("p=1; Path=/app", "https://example.com/app/start");
    assert!(jar.header_for_url("https://example.com/app/next").is_some());
    assert!(jar.header_for_url("https://example.com/other").is_none());
}

#[test]
fn max_age_zero_and_negative_kill_cookie() {
    let mut jar = CookieJar::new();
    jar.ingest("dead=1");
    assert_eq!(jar.get("dead"), Some("1"));
    jar.ingest("dead=; Max-Age=0");
    assert_eq!(jar.get("dead"), None);
    jar.ingest("dead2=x; Max-Age=-1");
    assert_eq!(jar.get("dead2"), None);
}

#[test]
fn expires_http_date_parses_and_expires() {
    let mut jar = CookieJar::new();
    jar.ingest("old=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(jar.get("old"), None);
    jar.ingest("fresh=1; Expires=Wed, 09 Jun 2100 10:18:14 GMT");
    assert_eq!(jar.get("fresh"), Some("1"));
    jar.ingest("junk=1; Expires=not-a-date");
    assert_eq!(
        jar.get("junk"),
        Some("1"),
        "invalid date attr is ignored per rfc6265, cookie stays session-scoped"
    );
    jar.ingest("junk2=1; Expires=");
    assert_eq!(
        jar.get("junk2"),
        Some("1"),
        "empty date attr is ignored, cookie stays session-scoped"
    );
}

#[test]
fn header_for_url_filters_cross_site() {
    let mut jar = CookieJar::new();
    jar.ingest_for_url("a=1", "https://one.test/x");
    jar.ingest_for_url("b=2", "https://two.test/y");
    let h1 = jar.header_for_url("https://one.test/z").expect("h1");
    assert!(h1.contains("a=1"));
    assert!(!h1.contains("b=2"));
    let h2 = jar.header_for_url("https://two.test/z").expect("h2");
    assert!(h2.contains("b=2"));
    assert!(!h2.contains("a=1"));
}

#[test]
fn jar_capacity_evicts_without_panic() {
    let mut jar = CookieJar::new();
    for i in 0..600 {
        jar.ingest(&format!("k{i}=v{i}"));
    }
    assert!(jar.len() <= 600);
    assert!(!jar.is_empty());
}

#[test]
fn bad_utf8_and_control_chars_survive() {
    let mut jar = CookieJar::new();
    jar.ingest("ok=va\x01lue; Path=/");
    jar.ingest("\u{7f}=x");
    jar.ingest("n; ame=v");
    assert!(!jar.is_empty());
}

#[test]
fn cookie_value_maxage_suffix_not_misparsed() {
    let mut jar = CookieJar::new();
    jar.ingest_scoped("        k=vMax-Age=", "example.com", "/");
    assert_eq!(jar.get_for_host("k", "example.com"), Some("vMax-Age="));
    let mut jar = CookieJar::new();
    jar.ingest_scoped(
        "  s=xExpires=Fri, 01 Jan 2027 00:00:00 GMT",
        "example.com",
        "/",
    );
    assert_eq!(
        jar.get_for_host("s", "example.com"),
        Some("xExpires=Fri, 01 Jan 2027 00:00:00 GMT")
    );
}

#[test]
fn cookie_name_value_trimmed_around_eq() {
    let mut jar = CookieJar::new();
    jar.ingest_scoped("sid =abc", "example.com", "/");
    assert_eq!(jar.get_for_host("sid", "example.com"), Some("abc"));
    jar.ingest_scoped("wide= spaced ", "example.com", "/");
    assert_eq!(jar.get_for_host("wide", "example.com"), Some("spaced"));
}

#[test]
fn cookie_header_order_path_length_desc() {
    let mut jar = CookieJar::new();
    jar.ingest_scoped("root=1; Path=/", "example.com", "/");
    jar.ingest_scoped("deep=2; Path=/a/b/c", "example.com", "/");
    jar.ingest_scoped("mid=3; Path=/a", "example.com", "/");
    let h = jar
        .header_for_url("https://example.com/a/b/c/page")
        .unwrap();
    let names: Vec<&str> = h
        .split("; ")
        .map(|kv| kv.split('=').next().unwrap())
        .collect();
    assert_eq!(names, vec!["deep", "mid", "root"]);
}
