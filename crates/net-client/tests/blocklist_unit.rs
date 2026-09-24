use net_client::blocklist::{is_blocked_host, url_blocked};

#[test]
fn known_trackers_blocked() {
    assert!(is_blocked_host("ad.doubleclick.net"));
    assert!(is_blocked_host("doubleclick.net"));
    assert!(is_blocked_host("www.google-analytics.com"));
    assert!(is_blocked_host("static.hotjar.com"));
}

#[test]
fn normal_hosts_pass() {
    assert!(!is_blocked_host("example.com"));
    assert!(!is_blocked_host("en.wikipedia.org"));
    assert!(!is_blocked_host(""));
}

#[test]
fn url_lookup() {
    assert!(url_blocked(
        "https://www.googletagmanager.com/gtag/js?id=G-1"
    ));
    assert!(!url_blocked("https://example.com/x"));
    assert!(!url_blocked("about:blank"));
}

#[test]
fn userinfo_and_case_bypass_blocked() {
    assert!(url_blocked("https://x@doubleclick.net/"));
    assert!(url_blocked("https://DOUBLECLICK.NET/"));
    assert!(url_blocked("http://evil.com@ad.doubleclick.net/"));
    assert!(url_blocked("https://Sub.Google-ANALYTICS.com/"));
    assert!(!url_blocked("https://x@example.com/"));
}

#[test]
fn percent_encoded_and_fullwidth_trackers_blocked() {
    assert!(
        url_blocked("https://ads-%74witter.com/x"),
        "percent-декод хоста"
    );
    assert!(
        url_blocked("https://ads.%74witter.com/x"),
        "encoded поддомен"
    );
    assert!(
        url_blocked("https://ads-%54WITTER.com/x"),
        "декод дал uppercase — лоуэркейс обязан идти ПОСЛЕ декода"
    );
    assert!(url_blocked("https://ads-twitter%2Ecom/x"), "%2E = '.'");
    assert!(
        url_blocked("https://ads-twitter。com/x"),
        "полноширинная точка"
    );

    assert!(url_blocked("https://ａｄｓ-ｔｗｉｔｔｅｒ。com/x"));

    assert!(!url_blocked("https://ex%61mple.com/x"));
    assert!(!url_blocked("https://www.example。com/x"));
}
