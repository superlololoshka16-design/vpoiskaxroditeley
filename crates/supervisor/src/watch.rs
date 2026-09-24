use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parser_pipeline::VersionMonitor;
use scc::HashMap;

use crate::flow::{VisitCtx, exec_script, session_for};

#[derive(Clone)]
pub(crate) struct WatchRec {
    pub probe_page: bool,
    pub script: bytes::Bytes,
    pub token: Option<compact_str::CompactString>,
    pub seen: std::time::Instant,
}

const WATCH_TTL: Duration = Duration::from_secs(6 * 3600);
const WATCH_CAP: usize = 512;

pub(crate) fn watch_rec(probe_page: bool, script: bytes::Bytes) -> WatchRec {
    WatchRec {
        probe_page,
        script,
        token: None,
        seen: std::time::Instant::now(),
    }
}

pub(crate) struct AstDiff {
    pub old_skel: u64,
    pub new_skel: u64,
    pub old_len: usize,
    pub new_len: usize,
    pub first_diff: usize,
}

struct Hex64(u64);

impl std::fmt::Display for Hex64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

pub(crate) fn watch_gc(
    watched: &HashMap<compact_str::CompactString, WatchRec>,
    cap: Option<usize>,
) {
    let mut victims: Vec<compact_str::CompactString> = Vec::new();
    watched.iter_sync(|k, v| {
        if v.seen.elapsed() > WATCH_TTL {
            victims.push(compact_str::CompactString::from(k.as_str()));
        }
        true
    });
    for v in victims.drain(..) {
        let _ = watched.remove_sync(&v);
    }
    let Some(cap) = cap else {
        return;
    };
    if watched.len() <= cap {
        return;
    }
    let excess = watched.len() - cap;
    let mut n = 0usize;
    watched.iter_sync(|k, _| {
        if n < excess {
            victims.push(compact_str::CompactString::from(k.as_str()));
            n += 1;
        }
        n < excess
    });
    for v in victims {
        let _ = watched.remove_sync(&v);
    }
}

pub(crate) fn watch_put(
    watched: &HashMap<compact_str::CompactString, WatchRec>,
    url: &str,
    rec: WatchRec,
) {
    let _ = watched.insert_sync(compact_str::CompactString::from(url), rec);
    if watched.len() > WATCH_CAP {
        watch_gc(watched, Some(WATCH_CAP));
    }
}

pub(crate) fn ast_diff(old: &[u8], new: &[u8]) -> AstDiff {
    let o = runtime_exec::normalize(old).ok();
    let n = runtime_exec::normalize(new).ok();
    let first_diff = match (&o, &n) {
        (Some(o), Some(n)) => o
            .src
            .as_bytes()
            .iter()
            .zip(n.src.as_bytes().iter())
            .position(|(&a, &b)| a != b)
            .unwrap_or(o.src.len().min(n.src.len())),
        _ => 0,
    };
    AstDiff {
        old_skel: o.as_ref().map_or(0, |o| o.skel),
        new_skel: n.as_ref().map_or(0, |n| n.skel),
        old_len: o.as_ref().map_or(old.len(), |o| o.src.len()),
        new_len: n.as_ref().map_or(new.len(), |n| n.src.len()),
        first_diff,
    }
}

pub(crate) fn note_build_change(
    mon: &VersionMonitor,
    url: &str,
    body: &[u8],
    alerts: &AtomicU64,
) -> bool {
    if mon.check(url, body) {
        let _ = alerts.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(url, "challenge build changed");
        true
    } else {
        false
    }
}

pub(crate) async fn watch_daemon(
    ctx: VisitCtx,
    watched: Arc<HashMap<compact_str::CompactString, WatchRec>>,
    monitor: Arc<VersionMonitor>,
    alerts: Arc<AtomicU64>,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;
        watch_gc(&watched, None);
        let mut probes: Vec<(compact_str::CompactString, WatchRec)> = Vec::new();
        watched.iter_sync(|url, rec| {
            probes.push((compact_str::CompactString::from(url.as_str()), rec.clone()));
            true
        });
        for (url, rec) in probes {
            let Ok((slot, mut session)) = session_for(&ctx, url.as_str(), None) else {
                continue;
            };
            let body = if rec.probe_page {
                match net_client::fetch_page(&ctx.engines, slot, &mut session, url.as_str()).await {
                    Ok(f) => f.page.challenge.clone().unwrap_or_default(),
                    Err(_) => bytes::Bytes::new(),
                }
            } else {
                match ctx.engines.client_for(slot).get(url.as_str()).send().await {
                    Ok(resp) => resp.bytes().await.unwrap_or_default(),
                    Err(_) => bytes::Bytes::new(),
                }
            };
            if body.is_empty() || !note_build_change(&monitor, url.as_str(), &body, &alerts) {
                continue;
            }
            let outcome = exec_script(
                &ctx.pool,
                &session.profile,
                url.as_str(),
                body.clone(),
                ctx.timeout,
                slot,
            )
            .await;
            let new_token = outcome.token;
            let oracle = match (&rec.token, &new_token) {
                (Some(old), Some(new)) => old == new,
                (None, Some(_)) => true,
                _ => false,
            };
            tracing::warn!(
                url = url.as_str(),
                oracle,
                token = new_token.as_deref().unwrap_or("-"),
                "challenge smoke test"
            );
            let diff = ast_diff(rec.script.as_ref(), body.as_ref());
            tracing::warn!(
                url = url.as_str(),
                old_skel = %Hex64(diff.old_skel),
                new_skel = %Hex64(diff.new_skel),
                old_len = diff.old_len,
                new_len = diff.new_len,
                first_diff = diff.first_diff,
                "challenge ast diff"
            );
            watch_put(
                &watched,
                url.as_str(),
                WatchRec {
                    probe_page: rec.probe_page,
                    script: body,
                    token: new_token,
                    seen: std::time::Instant::now(),
                },
            );
        }
    }
}
