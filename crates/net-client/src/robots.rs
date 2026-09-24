use compact_str::CompactString;
use core_utils::BytesExt as _;
use scc::HashMap;
use smallvec::SmallVec;

pub struct RobotsCache {
    cache: HashMap<CompactString, RobotsRules>,
}

#[derive(Debug, Clone, Default)]
struct RobotsRules {
    disallowed: SmallVec<[CompactString; 8]>,
    allowed: SmallVec<[CompactString; 8]>,
}

impl RobotsCache {
    pub fn new() -> Self {
        RobotsCache {
            cache: HashMap::new(),
        }
    }

    pub fn parse_and_store(&self, domain: &str, body: &str, our_agent: &str) {
        let rules = parse_robots_txt(body, our_agent);
        let _ = self.cache.insert_sync(CompactString::new(domain), rules);
    }

    pub fn is_cached(&self, domain: &str) -> bool {
        self.cache.contains_sync(domain)
    }

    pub fn store_empty(&self, domain: &str) {
        let _ = self
            .cache
            .insert_sync(CompactString::new(domain), RobotsRules::default());
    }

    pub fn is_allowed(&self, domain: &str, path: &str) -> bool {
        let mut verdict = true;
        let _ = self.cache.read_sync(domain, |_, rules| {
            let mut best: Option<(usize, bool)> = None;
            for (patterns, allow) in [(&rules.allowed, true), (&rules.disallowed, false)] {
                for pattern in patterns {
                    if path_matches(path, pattern.as_str())
                        && best.is_none_or(|(len, _)| pattern.len() > len)
                    {
                        best = Some((pattern.len(), allow));
                    }
                }
            }
            verdict = best.is_none_or(|(_, allow)| allow);
        });
        verdict
    }
}

fn parse_robots_txt(body: &str, our_agent: &str) -> RobotsRules {
    let agent_lower = our_agent.to_ascii_lowercase();
    let rules = parse_section(body, &agent_lower, true);
    if rules.has_specific_agent {
        return rules.rules;
    }
    parse_section(body, &agent_lower, false).rules
}

struct SectionParse {
    rules: RobotsRules,
    has_specific_agent: bool,
}

fn parse_section(body: &str, our_agent: &str, specific_only: bool) -> SectionParse {
    let mut rules = RobotsRules::default();
    let mut has_specific_agent = false;

    let mut group_matches = false;
    let mut last_was_ua = false;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().as_bytes();
        let value = value.trim();
        if key.eq_ci(b"user-agent") {
            if !last_was_ua {
                group_matches = false;
            }
            if value == "*" {
                if !specific_only {
                    group_matches = true;
                }
            } else {
                let a = value.as_bytes();
                let agent = our_agent.as_bytes();
                let token_match = a.len() == agent.len() && a.eq_ignore_ascii_case(agent)
                    || a.len() < agent.len()
                        && agent[..a.len()].eq_ignore_ascii_case(a)
                        && agent[a.len()] == b'/';
                if token_match {
                    group_matches = true;
                    has_specific_agent = true;
                }
            }
            last_was_ua = true;
        } else {
            if group_matches && !value.is_empty() {
                if key.eq_ci(b"disallow") {
                    rules.disallowed.push(CompactString::new(value));
                } else if key.eq_ci(b"allow") {
                    rules.allowed.push(CompactString::new(value));
                }
            }
            last_was_ua = false;
        }
    }
    SectionParse {
        rules,
        has_specific_agent,
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let (pat, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };

    let mut segs = pat.split('*');
    let Some(mut rest) = path.strip_prefix(segs.next().unwrap_or("")) else {
        return false;
    };
    let mut mids: SmallVec<[&str; 4]> = segs.collect();
    if anchored {
        match mids.pop() {
            Some(last) => {
                for seg in mids {
                    match rest.find(seg) {
                        Some(i) => rest = &rest[i + seg.len()..],
                        None => return false,
                    }
                }
                rest.ends_with(last)
            }

            None => rest.is_empty(),
        }
    } else {
        for seg in mids {
            match rest.find(seg) {
                Some(i) => rest = &rest[i + seg.len()..],
                None => return false,
            }
        }
        true
    }
}
