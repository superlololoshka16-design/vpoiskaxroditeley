use net_client::RobotsCache;

#[test]
fn empty_body_allows_all() {
    let c = RobotsCache::new();
    c.parse_and_store("example.com", "", "silo");
    assert!(c.is_allowed("example.com", "/anything"));
    assert!(c.is_allowed("unknown.host", "/x"));
}

#[test]
fn disallow_root_blocks_everything() {
    let c = RobotsCache::new();
    c.parse_and_store("example.com", "User-agent: *\nDisallow: /\n", "silo");
    assert!(!c.is_allowed("example.com", "/page"));
    assert!(c.is_allowed("other.com", "/page"));
}

#[test]
fn allow_overrides_disallow_prefix() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "example.com",
        "User-agent: *\nDisallow: /private\nAllow: /private/public\n",
        "silo",
    );
    assert!(!c.is_allowed("example.com", "/private/x"));
    assert!(c.is_allowed("example.com", "/private/public"));
    assert!(c.is_allowed("example.com", "/open"));
}

#[test]
fn longer_disallow_wins_over_shorter_allow() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "example.com",
        "User-agent: *\nDisallow: /a\nAllow: /\n",
        "silo",
    );
    assert!(!c.is_allowed("example.com", "/a"));
    assert!(c.is_allowed("example.com", "/b"));
}

#[test]
fn equal_length_tie_allows() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "example.com",
        "User-agent: *\nDisallow: /a\nAllow: /a\n",
        "silo",
    );
    assert!(c.is_allowed("example.com", "/a"));
}

#[test]
fn specific_agent_section_wins_over_star() {
    let body = "User-agent: *\nDisallow: /\n\nUser-agent: silo\nDisallow: /only-silo\n";
    let c = RobotsCache::new();
    c.parse_and_store("example.com", body, "silo");
    assert!(c.is_allowed("example.com", "/page"));
    assert!(!c.is_allowed("example.com", "/only-silo"));
}

#[test]
fn wildcard_and_dollar_patterns() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "example.com",
        "User-agent: *\nDisallow: /*.pdf$\nDisallow: /tmp\n",
        "silo",
    );
    assert!(!c.is_allowed("example.com", "/docs/report.pdf"));
    assert!(c.is_allowed("example.com", "/docs/report.pdfx"));
    assert!(!c.is_allowed("example.com", "/tmp/cache"));
}

#[test]
fn multi_wildcard_patterns_match_everywhere() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "multi.io",
        "User-agent: *\nDisallow: /*/private/*\n",
        "silo",
    );
    assert!(!c.is_allowed("multi.io", "/x/private/y"));
    assert!(!c.is_allowed("multi.io", "/a/b/private/c/d"));
    assert!(c.is_allowed("multi.io", "/public/x"));
    assert!(c.is_allowed("multi.io", "/private-no-slash"));
}

#[test]
fn anchored_multi_wildcard() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "anch.io",
        "User-agent: *\nDisallow: /*/x/*y$\nDisallow: /*.php$\n",
        "silo",
    );

    assert!(!c.is_allowed("anch.io", "/a/x/by"));
    assert!(!c.is_allowed("anch.io", "/a/x/xy"));

    assert!(c.is_allowed("anch.io", "/x/xx/xy"));

    assert!(c.is_allowed("anch.io", "/a/x/byz"));
    assert!(c.is_allowed("anch.io", "/a/bz"));

    assert!(!c.is_allowed("anch.io", "/forum/index.php"));
    assert!(c.is_allowed("anch.io", "/forum/index.php.bak"));
}

#[test]
fn wildcard_prefix_and_double_star() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "star.io",
        "User-agent: *\nDisallow: /*sensitive*/download\nDisallow: /a**b\n",
        "silo",
    );
    assert!(!c.is_allowed("star.io", "/xx/sensitive/yy/download"));
    assert!(c.is_allowed("star.io", "/xx/sensitive/yy/uploads"));

    assert!(!c.is_allowed("star.io", "/ab"));
    assert!(!c.is_allowed("star.io", "/a-anything-b"));
    assert!(c.is_allowed("star.io", "/b"));
}

#[test]
fn garbage_lines_do_not_panic() {
    let c = RobotsCache::new();
    c.parse_and_store(
        "example.com",
        "not a directive\n::\nUser-agent\nDisallow\n\n#comment\n",
        "silo",
    );
    assert!(c.is_allowed("example.com", "/x"));
}

#[test]
fn multi_ua_groups_apply_to_all_members() {
    let cache = RobotsCache::new();
    cache.parse_and_store(
        "multi.io",
        "User-agent: alpha\nUser-agent: beta\nDisallow: /group\n\nUser-agent: *\nDisallow: /all",
        "alpha",
    );
    assert!(
        !cache.is_allowed("multi.io", "/group/x"),
        "правило группы обязано действовать на alpha"
    );
    let cache = RobotsCache::new();
    cache.parse_and_store(
        "multi.io",
        "User-agent: alpha\nUser-agent: beta\nDisallow: /group\n\nUser-agent: *\nDisallow: /all",
        "beta",
    );
    assert!(!cache.is_allowed("multi.io", "/group/x"), "и на beta тоже");

    let cache = RobotsCache::new();
    cache.parse_and_store(
        "multi.io",
        "User-agent: alpha\nDisallow: /only-alpha\n\nUser-agent: *\nDisallow: /all",
        "alpha",
    );
    assert!(!cache.is_allowed("multi.io", "/only-alpha/x"));
    assert!(
        cache.is_allowed("multi.io", "/other"),
        "specific-группа не тянет за собой *-правила"
    );
}

#[test]
fn substring_agents_no_longer_match() {
    let cache = RobotsCache::new();
    cache.parse_and_store(
        "sub.io",
        "User-agent: bot\nDisallow: /banned",
        "silo-bot/1.0",
    );

    assert!(cache.is_allowed("sub.io", "/banned/x"));

    let cache = RobotsCache::new();
    cache.parse_and_store(
        "sub.io",
        "User-agent: silo-bot\nDisallow: /banned",
        "silo-bot/1.0",
    );
    assert!(!cache.is_allowed("sub.io", "/banned/x"));
}
