mod collector;
pub mod detector;
pub mod dom;
mod pipeline;
mod scratch;
mod telemetry;

pub use collector::{Limits, PageData};
pub use detector::{CaptchaFamily, detect_widget, extract_sitekey_from_url, sitekey_passes};
pub use dom::{ATTR_NAMES, DomTree};
pub use pipeline::{Flow, PipeError, StreamPipeline};
pub use telemetry::{TelemetryProvider, TelemetryRoute, Transport, detect_route};

use std::sync::atomic::{AtomicU64, Ordering};

pub const BYTE_BRAKE: u64 = 4 * 1024 * 1024;

pub fn validate_selectors(sels: &[(String, String)]) -> Result<(), String> {
    for (name, sel) in sels {
        sel.parse::<lol_html::Selector>()
            .map_err(|e| format!("{name} ({sel}): {e}"))?;
    }
    Ok(())
}

pub struct VersionMonitor {
    hashes: scc::HashMap<u64, AtomicU64>,
}

impl VersionMonitor {
    pub fn new() -> Self {
        Self {
            hashes: scc::HashMap::new(),
        }
    }

    pub fn check(&self, url: &str, content: &[u8]) -> bool {
        let key = core_utils::xxh3::hash(url.as_bytes());
        let new_hash = core_utils::xxh3::hash(content);
        let changed = self
            .hashes
            .read_sync(&key, |_, v| v.load(Ordering::Relaxed) != new_hash)
            .unwrap_or(true);
        if changed {
            let _ = self.hashes.insert_sync(key, AtomicU64::new(new_hash));
        }
        changed
    }
}
