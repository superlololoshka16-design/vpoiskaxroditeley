use core_utils::StrExt as _;

#[test]
fn host_strips_userinfo_and_port_and_lowercases() {
    assert_eq!(
        "https://user:pass@example.com/x".host().as_str(),
        "example.com"
    );
    assert_eq!("https://EXAMPLE.com:8443/x".host().as_str(), "example.com");
    assert_eq!("http://Example.COM".host().as_str(), "example.com");

    assert_eq!("https://alice@host.io/p".host().as_str(), "host.io");
}

#[test]
fn host_knows_non_http_schemes() {
    assert_eq!("socks5://1.2.3.4:1080".host().as_str(), "1.2.3.4");
    assert_eq!(
        "ftp://files.example.net/pub".host().as_str(),
        "files.example.net"
    );
}

#[test]
fn host_ipv6_without_brackets() {
    assert_eq!("http://[2001:db8::1]:443/x".host().as_str(), "2001:db8::1");
    assert_eq!("http://[::1]/x".host().as_str(), "::1");
    assert_eq!(
        "http://[2001:db8::1]:443/x".host_port().as_str(),
        "[2001:db8::1]:443"
    );
}

#[test]
fn host_port_browser_semantics() {
    assert_eq!(
        "https://example.com:443/x".host_port().as_str(),
        "example.com"
    );
    assert_eq!(
        "http://example.com:80/x".host_port().as_str(),
        "example.com"
    );
    assert_eq!(
        "https://example.com:8443/x".host_port().as_str(),
        "example.com:8443"
    );
    assert_eq!(
        "http://example.com:8080/x".host_port().as_str(),
        "example.com:8080"
    );

    assert_eq!("https://example.com/x".host_port().as_str(), "example.com");
}

#[test]
fn origin_parts_exclude_userinfo_and_normalize_port() {
    let (origin, host) = "https://user:pass@example.com:8443/x".origin_parts();
    assert_eq!(origin.as_str(), "https://example.com:8443");
    assert_eq!(host.as_str(), "example.com");

    let (origin, host) = "https://example.com:443/x".origin_parts();
    assert_eq!(origin.as_str(), "https://example.com");
    assert_eq!(host.as_str(), "example.com");
    let (origin, _) = "https://example.com/x".origin_parts();
    assert_eq!(origin.as_str(), "https://example.com");
}

#[test]
fn origin_brackets_ipv6_host() {
    let (origin, host) = "http://[2001:db8::1]:8080/x".origin_parts();
    assert_eq!(origin.as_str(), "http://[2001:db8::1]:8080");
    assert_eq!(host.as_str(), "2001:db8::1");
    let (origin, _) = "https://[::1]/x".origin_parts();
    assert_eq!(origin.as_str(), "https://[::1]");
}

#[test]
fn href_parts_split_tail() {
    let p = "https://example.com/a/b?x=1#frag".href_parts();
    assert_eq!(p.origin.as_str(), "https://example.com");
    assert_eq!(p.host.as_str(), "example.com");
    assert_eq!(p.path.as_str(), "/a/b");
    assert_eq!(p.search.as_str(), "?x=1");
    assert_eq!(p.hash.as_str(), "#frag");

    let p = "https://example.com".href_parts();
    assert_eq!(p.path.as_str(), "/");
    assert_eq!(p.search.as_str(), "");
    assert_eq!(p.hash.as_str(), "");
}

#[test]
fn path_and_empty_hosts() {
    assert_eq!(core_utils::path_of("https://example.com"), "/");
    assert_eq!(core_utils::path_of("https://example.com/a?b"), "/a?b");
    assert_eq!("not-a-url".host().as_str(), "not-a-url");
    assert_eq!("https://".host().as_str(), "");
}

#[test]
fn join_origin_keeps_authority() {
    assert_eq!(
        core_utils::join_origin("https://example.com/a/b", "/c", false).as_str(),
        "https://example.com/c"
    );
    assert_eq!(
        core_utils::join_origin("https://example.com", "x/y", true).as_str(),
        "https://example.com/x/y"
    );

    assert_eq!(
        core_utils::join_origin("https://example.com/a", "//cdn.io/lib", false).as_str(),
        "https://cdn.io/lib"
    );

    assert_eq!(
        core_utils::join_origin("https://example.com/a", "/challenge", true).as_str(),
        "https://example.com/challenge"
    );
    assert_eq!(
        core_utils::join_origin("https://example.com/a", "rel", true).as_str(),
        "https://example.com/rel"
    );
}
