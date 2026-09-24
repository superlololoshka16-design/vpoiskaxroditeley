use aho_corasick::AhoCorasick;
use compact_str::CompactString;
use std::sync::LazyLock;

use crate::detector::build_ac;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryProvider {
    Arkose,
    DataDome,
    Turnstile,
    HCaptcha,
    ReCaptcha,
    InHouse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    CdnPost,
    FormField,
    CustomHeader,
}

#[derive(Clone, Copy)]
pub(crate) struct RouteSpec {
    pub provider: TelemetryProvider,
    pub endpoint: &'static str,
    pub transport: Transport,
    pub field: &'static str,
}

#[derive(Debug, Clone)]
pub struct TelemetryRoute {
    pub provider: TelemetryProvider,
    pub endpoint: CompactString,
    pub transport: Transport,
    pub field: CompactString,
}

static INLINE_HINTS: &[&str] = &[
    "/telemetry",
    "/api/telemetry",
    "/beacon",
    "/collect?v=",
    "sendBeacon(",
];

fn inline_path(idx: usize) -> &'static str {
    match INLINE_HINTS[idx] {
        "/collect?v=" => "/collect",
        "sendBeacon(" => "/beacon",
        path => path,
    }
}

static ROUTES: LazyLock<(AhoCorasick, Vec<&'static crate::detector::NeedleRow>)> =
    LazyLock::new(|| {
        let rows: Vec<_> = crate::detector::NEEDLE_ROWS
            .iter()
            .filter(|r| r.route.is_some())
            .collect();
        let ac = build_ac(rows.iter().map(|r| r.needle), false);
        (ac, rows)
    });

static INLINE_AC: LazyLock<AhoCorasick> =
    LazyLock::new(|| build_ac(INLINE_HINTS.iter().map(|s| s.as_bytes()), false));

pub fn detect_route(scripts: &[&str], inline: &[u8]) -> Option<TelemetryRoute> {
    let (ac, rows) = &*ROUTES;
    let mut hits: smallvec::SmallVec<[usize; 4]> = smallvec::SmallVec::new();
    for src in scripts {
        for mat in ac.find_iter(src.as_bytes()) {
            let idx = mat.pattern().as_usize();
            if !hits.contains(&idx) {
                hits.push(idx);
            }
        }
    }
    for &priority in crate::detector::PRIORITY {
        for &idx in &hits {
            let row = rows[idx];
            if row.family == priority
                && let Some(spec) = row.route
            {
                return Some(TelemetryRoute {
                    provider: spec.provider,
                    endpoint: CompactString::const_new(spec.endpoint),
                    transport: spec.transport,
                    field: CompactString::const_new(spec.field),
                });
            }
        }
    }
    if !inline.is_empty()
        && let Some(mat) = INLINE_AC.find(inline)
    {
        return Some(TelemetryRoute {
            provider: TelemetryProvider::InHouse,
            endpoint: CompactString::const_new(inline_path(mat.pattern().as_usize())),
            transport: Transport::CdnPost,
            field: CompactString::const_new("telemetry"),
        });
    }
    None
}
