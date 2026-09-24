use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use challenge_solver::anubis as anubis_solver;
use compact_str::CompactString;
use core_utils::StrExt as _;
use core_utils::xxh3;
use net_client::{EngineSet, Fetched, RobotsCache, fetch_page_sel, pass_anubis};
use payload_gen::input::interaction_events_for;
use runtime_exec::{
    ExecKind, ExecOutcome, ExecReq, NAV_HOP_MAX, ProfileSnap, WorkerPool, nav_target,
};
use session_state::{Profile, ProxyConfig, Session};
use smallvec::SmallVec;

use crate::ms;
use crate::site_key;
use crate::site_override::SiteOverrides;
use crate::stats::StatsRef;

pub fn proxy_cfg(raw: &str) -> Result<ProxyConfig, String> {
    ProxyConfig::parse(raw).map_err(|e| format!("proxy url rejected: {e}"))
}

pub fn slot_bound_to_proxy(profiles: &[Arc<Profile>], cfg: &ProxyConfig) -> Option<usize> {
    profiles.iter().position(|p| {
        p.proxy.as_ref().is_some_and(|px| {
            px.target == cfg.target
                && px.port == cfg.port
                && px.scheme == cfg.scheme
                && px.auth == cfg.auth
                && px.asn == cfg.asn
                && px.utc_offset == cfg.utc_offset
        })
    })
}

pub fn reslot_overridden(
    base: &Profile,
    url: &str,
    asn: u32,
    overrides: &SiteOverrides,
) -> Arc<Profile> {
    let host = core_utils::host_of(url);
    let profile = net_client::reslot_with_asn(base, asn);
    match overrides.for_host(host.as_str()) {
        Some(ov) => Arc::new(ov.apply_validated(profile.as_ref())),
        None => profile,
    }
}

pub fn profile_for_host_with_overrides(
    pool: &[Arc<Profile>],
    url: &str,
    asn: u32,
    overrides: &SiteOverrides,
) -> (usize, Arc<Profile>) {
    let slot = crate::catalog_slot_of(core_utils::host_of(url).as_str(), pool.len());
    let profile = reslot_overridden(&pool[slot], url, asn, overrides);
    (slot, profile)
}

pub fn profile_for_task(
    ctx: &VisitCtx,
    profile_id: Option<u64>,
    proxy: Option<&str>,
) -> Result<(usize, Arc<Profile>), String> {
    let base = match profile_id {
        Some(pid) => ctx
            .profiles
            .get(pid as usize)
            .cloned()
            .ok_or("profile not found")?,
        None => ctx.profiles.first().cloned().ok_or("no profile")?,
    };
    let default_slot = profile_id.map_or(0, |pid| (pid as usize) % ctx.profiles.len().max(1));
    match proxy {
        None => Ok((default_slot, base)),
        Some(raw) => {
            let cfg = proxy_cfg(raw)?;
            match slot_bound_to_proxy(&ctx.profiles, &cfg) {
                Some(slot) => Ok((slot, Arc::clone(&ctx.profiles[slot]))),
                None => Ok((
                    default_slot,
                    net_client::reslot_with_proxy(base.as_ref(), ctx.asn, cfg),
                )),
            }
        }
    }
}

#[derive(Clone)]
pub struct VisitCtx {
    pub engines: Arc<EngineSet>,
    pub pool: Arc<WorkerPool>,
    pub stats: StatsRef,
    pub profiles: Vec<Arc<Profile>>,
    pub asn: u32,
    pub timeout: Duration,
    pub overrides: Arc<SiteOverrides>,
    pub robots: Arc<RobotsCache>,
}

pub struct HopOutcome {
    pub url: CompactString,
    pub slot: usize,
    pub fetched: Fetched,
    pub session: Session,
    pub token: Option<CompactString>,
    pub nav: Option<CompactString>,
    pub anubis: Option<Result<AnubisFlow, String>>,
}

impl HopOutcome {
    pub fn anubis_terminal(&self) -> bool {
        anubis_terminal_state(self.anubis.as_ref(), &self.fetched)
    }
}

pub struct VisitOutcome {
    pub hops: Vec<HopOutcome>,
    pub error: Option<String>,
}

pub struct ChallengeOutcome {
    pub token: Option<CompactString>,
    pub nav: Option<CompactString>,
}

pub struct AnubisFlow {
    pub challenge: anubis_solver::AnubisChallenge,
    pub answer: sonic_rs::Value,
    pub pass: Result<net_client::AnubisPass, String>,
    pub refetch: Result<Fetched, String>,
    pub solve_ms: u64,
}

impl AnubisFlow {
    pub fn passed(&self) -> bool {
        self.refetch
            .as_ref()
            .is_ok_and(|f2| f2.page.anubis.is_none())
    }
}

fn anubis_terminal_state(anubis: Option<&Result<AnubisFlow, String>>, fetched: &Fetched) -> bool {
    anubis.is_some_and(|a| a.as_ref().is_ok_and(|fl| fl.passed()))
        && fetched.page.challenge.is_none()
}

pub fn js_req(
    script: Bytes,
    snap: ProfileSnap,
    timeout: Duration,
    slot: usize,
    kind: ExecKind,
    domain: u64,
) -> ExecReq {
    let mut req = ExecReq::bare(domain, script, snap, timeout, kind);
    req.net_slot = slot;
    req
}

fn with_input(mut req: ExecReq, profile: &Profile, href: &str, trust: i32) -> ExecReq {
    req.input = Some(interaction_events_for(profile, href, trust).into_iter().collect());
    req
}

pub async fn exec_script(
    pool: &WorkerPool,
    profile: &Arc<Profile>,
    href: &str,
    script: Bytes,
    timeout: Duration,
    slot: usize,
) -> ExecOutcome {
    let domain = xxh3::hash(script.as_ref());
    let req = js_req(
        script,
        ProfileSnap::from_parts(profile, href, ""),
        timeout,
        slot,
        ExecKind::Js,
        domain,
    );
    pool.exec(with_input(req, profile, href, 0)).await
}

pub async fn run_challenge(
    ctx: &VisitCtx,
    session: &mut Session,
    f: &Fetched,
    slot: usize,
) -> Option<ChallengeOutcome> {
    let script = f.page.challenge.clone()?;
    ctx.stats.add_script();
    let cookie = session
        .jar
        .header_for_url(f.uri.as_str())
        .unwrap_or_default();
    let snap = ProfileSnap::from_parts(&session.profile, f.uri.as_str(), cookie.as_str())
        .with_rtt(f.elapsed_ms.min(u32::MAX as u64) as u32);
    let mut req = js_req(
        script,
        snap,
        ctx.timeout,
        slot,
        ExecKind::Js,
        site_key(f.uri.as_str()),
    );
    req.doc = Some(Arc::clone(&f.page));
    req.script_node = f.page.challenge_node;
    let req = with_input(req, session.profile.as_ref(), f.uri.as_str(), session.trust());
    let outcome = ctx.pool.exec(req).await;
    if let Some(line) = outcome.cookie_out.as_deref() {
        let origin = f.uri.as_str();
        let host = session.host();
        let path = core_utils::path_of(origin);
        for kv in line.split(';') {
            session.jar.ingest_scoped(kv.trim(), host.as_str(), path);
        }
    }
    session.record_challenge(outcome.token.is_some());
    Some(ChallengeOutcome {
        token: outcome.token,
        nav: outcome.nav,
    })
}

pub async fn exec_anubis(
    pool: &WorkerPool,
    raw: bytes::Bytes,
    snap: ProfileSnap,
    timeout: Duration,
    net_slot: usize,
) -> Result<anubis_solver::SolvedAnubis, String> {
    let domain = xxh3::hash(raw.as_ref());
    let req = js_req(raw, snap, timeout, net_slot, ExecKind::Anubis, domain);
    let outcome = pool.exec(req).await;
    match outcome.anubis {
        Some(sol) => Ok(sol),
        None => Err(outcome
            .err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "anubis solve failed".into())),
    }
}

pub async fn solve_anubis(
    ctx: &VisitCtx,
    raw: bytes::Bytes,
    href: &str,
    profile: &Arc<Profile>,
    slot: usize,
) -> Result<(anubis_solver::AnubisChallenge, anubis_solver::SolvedAnubis), String> {
    let ch = anubis_solver::AnubisChallenge::parse(&raw).map_err(|e| format!("{e:?}"))?;
    ctx.stats.add_script();
    let sol = exec_anubis(
        &ctx.pool,
        raw,
        ProfileSnap::from_parts(profile, href, ""),
        ctx.timeout,
        slot,
    )
    .await?;
    Ok((ch, sol))
}

pub async fn fetch_counted(
    ctx: &VisitCtx,
    slot: usize,
    session: &mut Session,
    url: &str,
    selectors: &[(String, String)],
) -> Result<Fetched, String> {
    let f = fetch_page_sel(&ctx.engines, slot, session, url, selectors)
        .await
        .map_err(|e| e.to_string())?;
    ctx.stats.add_fetch(f.bytes_in);
    Ok(f)
}

pub fn anubis_answer_value(sol: &anubis_solver::SolvedAnubis) -> sonic_rs::Value {
    sonic_rs::json!({
        "nonce": sol.nonce,
        "response": std::str::from_utf8(&sol.response_hex).unwrap_or_default(),
        "elapsedTime": sol.elapsed_time_ms,
        "startMs": sol.start_time_ms,
        "endMs": sol.end_time_ms,
    })
}

pub async fn anubis_page_flow(
    ctx: &VisitCtx,
    session: &mut Session,
    url: &str,
    f: &Fetched,
    anub: &[u8],
    slot: usize,
) -> Result<AnubisFlow, String> {
    let t0 = Instant::now();
    let (ch, sol) = solve_anubis(
        ctx,
        bytes::Bytes::copy_from_slice(anub),
        url,
        &session.profile,
        slot,
    )
    .await?;
    let real_solve_ms = t0.elapsed().as_millis() as f64;
    let emu_ms = sol.elapsed_time_ms;
    let hold_ms = (emu_ms - real_solve_ms).max(0.0).min(30_000.0);
    if hold_ms > 1.0 {
        tokio::time::sleep(std::time::Duration::from_millis(hold_ms as u64)).await;
    }
    let (origin, _) = core_utils::origin_of(f.uri.as_str());
    let mut pass = String::with_capacity(384);
    anubis_solver::build_pass_url(
        origin.as_str(),
        &ch.id,
        &sol.response_hex,
        sol.nonce,
        sol.elapsed_time_ms,
        url,
        &mut pass,
    );
    let pass_result = pass_anubis(&ctx.engines, slot, session, &pass, url)
        .await
        .map_err(|e| e.to_string());
    let refetch = fetch_counted(ctx, slot, session, url, &[]).await;
    Ok(AnubisFlow {
        solve_ms: ms(t0, Instant::now()),
        answer: anubis_answer_value(&sol),
        challenge: ch,
        pass: pass_result,
        refetch,
    })
}

pub fn profile_for_with_proxy(
    ctx: &VisitCtx,
    url: &str,
    proxy: Option<&str>,
) -> Result<(usize, Arc<Profile>), String> {
    let Some(raw) = proxy else {
        return Ok(profile_for_host_with_overrides(
            &ctx.profiles,
            url,
            ctx.asn,
            &ctx.overrides,
        ));
    };
    let cfg = proxy_cfg(raw)?;
    match slot_bound_to_proxy(&ctx.profiles, &cfg) {
        Some(i) => Ok((
            i,
            reslot_overridden(ctx.profiles[i].as_ref(), url, ctx.asn, &ctx.overrides),
        )),
        None => Err(format!(
            "proxy {} not bound in catalog (start with --proxy)",
            cfg.redacted_string()
        )),
    }
}

pub fn session_for(
    ctx: &VisitCtx,
    url: &str,
    proxy: Option<&str>,
) -> Result<(usize, Session), String> {
    let (slot, profile) = profile_for_with_proxy(ctx, url, proxy)?;
    Ok((slot, Session::new(profile, url)))
}

static ROBOTS_ON: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| core_utils::env_flag("SILO_ROBOTS"));

#[inline]
fn robots_enabled() -> bool {
    *ROBOTS_ON
}

async fn robots_allows(ctx: &VisitCtx, url: &str, slot: usize) -> bool {
    let host = core_utils::host_of(url);
    if host.is_empty() {
        return true;
    }
    if !ctx.robots.is_cached(host.as_str()) {
        let robots_url = url.origin_parts().0.join_origin("/robots.txt", false);
        let mut body = String::new();
        if let Ok(resp) = ctx
            .engines
            .client_for(slot)
            .get(robots_url.as_str())
            .timeout(Duration::from_secs(10))
            .send()
            .await
            && resp.status().is_success()
            && let Ok(text) = resp.text().await
        {
            body = text;
        }
        if body.is_empty() {
            ctx.robots.store_empty(host.as_str());
        } else {
            ctx.robots.parse_and_store(host.as_str(), &body, "silo");
        }
    }
    let path = core_utils::path_of(url);
    ctx.robots.is_allowed(host.as_str(), path)
}

pub async fn visit(
    ctx: &VisitCtx,
    url: &str,
    selectors: &[(String, String)],
    proxy: Option<&str>,
) -> VisitOutcome {
    let mut hops: Vec<HopOutcome> = Vec::new();
    let mut url = CompactString::from(url);
    let mut depth = 0u8;
    let mut reslot_cache: SmallVec<[(usize, Arc<Profile>); 4]> = SmallVec::new();
    let mut session = Session::new(Arc::clone(&ctx.profiles[0]), url.as_str());
    macro_rules! bail {
        ($e:expr) => {
            return VisitOutcome {
                hops,
                error: Some($e),
            }
        };
    }
    loop {
        if depth > NAV_HOP_MAX {
            bail!("navigation hop limit exceeded".into());
        }
        let (slot, profile) = match proxy {
            None => {
                let slot = crate::catalog_slot_of(
                    core_utils::host_of(url.as_str()).as_str(),
                    ctx.profiles.len(),
                );
                let profile = match reslot_cache.iter().find(|(s, _)| *s == slot) {
                    Some((_, cached)) => Arc::clone(cached),
                    None => {
                        let fresh = reslot_overridden(
                            ctx.profiles[slot].as_ref(),
                            url.as_str(),
                            ctx.asn,
                            &ctx.overrides,
                        );
                        reslot_cache.push((slot, Arc::clone(&fresh)));
                        fresh
                    }
                };
                (slot, profile)
            }
            Some(p) => match profile_for_with_proxy(ctx, url.as_str(), Some(p)) {
                Ok(sel) => sel,
                Err(e) => bail!(e),
            },
        };
        session.rebase(profile, url.as_str());
        if robots_enabled() && !robots_allows(ctx, url.as_str(), slot).await {
            bail!("robots.txt disallows this path".into());
        }
        let fetched = match fetch_counted(ctx, slot, &mut session, url.as_str(), selectors).await {
            Ok(f) => f,
            Err(e) => bail!(e),
        };
        let mut anubis = None;
        if let Some(anub) = fetched.page.anubis.as_deref() {
            anubis =
                Some(anubis_page_flow(ctx, &mut session, url.as_str(), &fetched, anub, slot).await);
        }
        let (token, nav) = if anubis_terminal_state(anubis.as_ref(), &fetched) {
            (None, None)
        } else {
            run_challenge(ctx, &mut session, &fetched, slot)
                .await
                .map(|o| (o.token, o.nav))
                .unwrap_or_default()
        };
        let target = nav
            .as_deref()
            .map(|n| CompactString::from(nav_target(n, url.as_str())));
        let profile = session.profile.clone();
        let hop_url = url.clone();
        let next_session = Session::new(profile, hop_url.as_str());
        hops.push(HopOutcome {
            url: hop_url,
            slot,
            fetched,
            session: std::mem::replace(&mut session, next_session),
            token,
            nav,
            anubis,
        });
        match target {
            Some(t) => {
                url = t;
                depth += 1;
            }
            None => return VisitOutcome { hops, error: None },
        }
    }
}
