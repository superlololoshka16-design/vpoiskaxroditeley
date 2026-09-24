use compact_str::CompactString;
use core_utils::BytesExt as _;
use core_utils::{FxBuild, fx_map};
use session_state::{Platform, Profile};
use sonic_rs::{JsonContainerTrait as _, JsonValueTrait as _};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct SiteOverride {
    pub ua: Option<Arc<str>>,
    pub sec_ch_ua: Option<Arc<str>>,
    pub accept_language: Option<Arc<str>>,
    pub locale: Option<CompactString>,
    pub tz: Option<CompactString>,
    pub platform: Option<Platform>,
}

fn platform_of(p: &str) -> Option<Platform> {
    let b = p.as_bytes();
    [
        (b"windows".as_slice(), Platform::Windows),
        (b"macos".as_slice(), Platform::MacOS),
        (b"linux".as_slice(), Platform::Linux),
        (b"android".as_slice(), Platform::Android),
    ]
    .into_iter()
    .find(|(k, _)| b.eq_ci(k))
    .map(|(_, p)| p)
}

impl SiteOverride {
    pub fn from_json(v: &sonic_rs::Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let get = |k: &str| v.get(k).and_then(|x| x.as_str());
        let platform = get("platform").and_then(platform_of);
        let out = Self {
            ua: get("ua").map(Arc::from),
            sec_ch_ua: get("secChUa").map(Arc::from),
            accept_language: get("acceptLanguage").map(Arc::from),
            locale: get("locale").map(CompactString::new),
            tz: get("tz").map(CompactString::new),
            platform,
        };
        if matches!(
            out,
            Self {
                ua: None,
                sec_ch_ua: None,
                accept_language: None,
                locale: None,
                tz: None,
                platform: None,
            }
        ) {
            return None;
        }
        Some(out)
    }

    pub fn apply_validated(&self, base: &Profile) -> Profile {
        let next = self.apply(base);
        if next.validate().is_ok() {
            return next;
        }
        tracing::warn!(target: "supervisor", "site override produced an inconsistent profile, ignored");
        Profile::clone(base)
    }

    pub fn apply(&self, base: &Profile) -> Profile {
        let mut next = Profile::clone(base);
        if let Some(ua) = &self.ua {
            next.ua = Arc::clone(ua);
        }
        if let Some(sec) = &self.sec_ch_ua {
            next.sec_ch_ua = Arc::clone(sec);
        }
        if let Some(lang) = &self.accept_language {
            next.accept_language = Arc::clone(lang);
        }
        if let Some(locale) = &self.locale {
            next.locale = locale.clone();
        }
        if let Some(tz) = &self.tz {
            next.tz = tz.clone();
        }
        if let Some(platform) = self.platform {
            next.platform = platform;
            next.rebind_identity();
        }
        next
    }
}

#[derive(Clone)]
pub struct SiteOverrides {
    map: HashMap<u64, SiteOverride, FxBuild>,
}

impl SiteOverrides {
    pub fn empty() -> Self {
        Self { map: fx_map() }
    }

    pub fn from_json(v: &sonic_rs::Value) -> Self {
        let mut map: HashMap<u64, SiteOverride, FxBuild> = fx_map();
        if let Some(obj) = v.as_object() {
            for (host, spec) in obj.iter() {
                if let Some(ov) = SiteOverride::from_json(spec) {
                    map.insert(crate::site_key(host), ov);
                }
            }
        }
        Self { map }
    }

    pub fn from_env() -> Self {
        match std::env::var("SILO_SITE_OVERRIDES") {
            Ok(raw) if !raw.trim().is_empty() => match sonic_rs::from_str(raw.as_str()) {
                Ok(v) => Self::from_json(&v),
                Err(e) => {
                    tracing::warn!(target = "supervisor", "bad SILO_SITE_OVERRIDES json: {e}");
                    Self::empty()
                }
            },
            _ => Self::empty(),
        }
    }

    pub fn for_host(&self, host: &str) -> Option<&SiteOverride> {
        self.map.get(&crate::site_key(host))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}
