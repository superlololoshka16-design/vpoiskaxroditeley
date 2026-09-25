use supervisor::api;
use supervisor::flow::{self, VisitCtx, profile_for_host_with_overrides};
use supervisor::ms;
use supervisor::site_override::SiteOverrides;
use supervisor::stats::{StatBlock, StatsRef, p50p99, render_rss};

use clap::{Parser, Subcommand};
use net_client::{EngineSet, Fetched, engine_catalog_with_proxies};
use parser_pipeline::PageData;
use runtime_exec::{Bundle, Event, EventTx, WorkerPool, nav_target};
use session_state::{Profile, Session};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const POLYFILL_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/polyfill.js");
const POLYFILL_EMBED: &str = include_str!("../assets/polyfill.js");

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

#[cfg(windows)]
#[inline]
fn raise_timer_resolution() {
    unsafe {
        timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
#[inline]
fn raise_timer_resolution() {}

#[derive(Parser)]
#[command(name = "my-engine", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Run {
        #[arg(long, required = true)]
        url: Vec<String>,
        #[arg(long, default_value_t = 1)]
        workers: usize,
        #[arg(long, default_value_t = 150)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 0)]
        asn: u32,
        #[arg(long)]
        proxy: Vec<String>,
    },
    Bench {
        #[arg(long, required = true)]
        url: String,
        #[arg(long, default_value_t = 32)]
        n: usize,
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
        #[arg(long, default_value_t = 0)]
        asn: u32,
        #[arg(long)]
        proxy: Vec<String>,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1:8817")]
        listen: String,
        #[arg(long, default_value_t = 4)]
        workers: usize,
        #[arg(long, default_value_t = 150)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 0)]
        asn: u32,
        #[arg(long)]
        proxy: Vec<String>,
    },
}

fn render_visit(out: &mut std::io::StdoutLock<'_>, visit: &flow::VisitOutcome) {
    for hop in &visit.hops {
        render_hop(out, hop);
        if let Some(nav) = hop.nav.as_deref() {
            let target = nav_target(nav, hop.url.as_str());
            let _ = writeln!(out, "  navigation -> {target}");
        }
    }
    if let Some(e) = &visit.error {
        let _ = writeln!(out, "fetch failed: {e}");
    }
}

fn render_hop(out: &mut std::io::StdoutLock<'_>, hop: &flow::HopOutcome) {
    let terminal = hop.anubis_terminal();
    if let Some(anub) = &hop.anubis {
        match anub {
            Ok(fl) => {
                let _ = writeln!(
                    out,
                    "  anubis challenge: version={} diff={} algo={}",
                    hop.fetched.page.anubis_version.as_deref().unwrap_or("-"),
                    fl.challenge.difficulty,
                    fl.challenge.algorithm.name()
                );
                let _ = writeln!(
                    out,
                    "  anubis solved in {}ms (report: {})",
                    fl.solve_ms, fl.answer
                );
                match &fl.pass {
                    Ok(r) => {
                        let _ = writeln!(
                            out,
                            "  anubis pass-challenge: status={} hops={} auth_cookie={} final={}",
                            r.status,
                            r.hops,
                            r.auth_cookie.as_deref().unwrap_or("-"),
                            r.final_uri.as_str()
                        );
                    }
                    Err(e) => {
                        let _ = writeln!(out, "  anubis pass-challenge failed: {e}");
                    }
                }
                match &fl.refetch {
                    Ok(f2) => {
                        render_page(out, &f2.page);
                        let _ = writeln!(
                            out,
                            "  anubis gate: {}",
                            if fl.passed() {
                                "PASSED"
                            } else {
                                "STILL CHALLENGED"
                            }
                        );
                    }
                    Err(e) => {
                        let _ = writeln!(out, "  refetch failed: {e}");
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(out, "  anubis failed: {e}");
            }
        }
    }
    if terminal {
        return;
    }
    render_page(out, &hop.fetched.page);
    if hop.nav.is_none() && hop.fetched.page.challenge.is_some() {
        match hop.token.as_deref() {
            Some(tok) => {
                let _ = writeln!(out, "  challenge -> {tok}");
            }
            None => {
                let _ = writeln!(out, "  challenge failed");
            }
        }
    }
}

fn render_page(out: &mut std::io::StdoutLock<'_>, page: &PageData) {
    let _ = writeln!(
        out,
        "page: {} ({} bytes, {} forms, {} tokens, {} scripts, inline={}, dom_nodes={} dom_pool={})",
        page.title.as_deref().unwrap_or("-"),
        page.bytes_fed,
        page.forms.len(),
        page.tokens.len(),
        page.script_srcs.len(),
        page.inline_count,
        page.dom.len(),
        page.dom.pool_len()
    );
    if let Some(nd) = &page.next_data {
        let _ = writeln!(out, "  next_data: page={} build={}", nd.page, nd.build_id);
    }
    if let Some(f) = page.captcha_family {
        let fam = parser_pipeline::CaptchaFamily::from_idx(f);
        let _ = write!(out, "  captcha: {}", fam.as_str());
        if let Some(k) = &page.captcha_sitekey {
            let _ = write!(out, " sitekey={}", k.as_str());
        }
        let _ = writeln!(out);
    }
    for t in &page.tokens {
        let _ = writeln!(out, "  token: {t}");
    }
    for src in &page.script_srcs {
        let _ = writeln!(out, "  script src: {src}");
    }
    if page.utf8_bad_chunks > 0 || page.truncated || page.parse_errors > 0 {
        let _ = writeln!(
            out,
            "  [warn] utf8_bad={} truncated={} parse_err={}",
            page.utf8_bad_chunks, page.truncated, page.parse_errors
        );
    }
}

fn render_footer(out: &mut std::io::StdoutLock<'_>, stats: &StatBlock) {
    stats.render(out);
    render_rss(out);
}

fn spawn_event_drain(rx: crossbeam_channel::Receiver<Event>, stats: StatsRef) {
    std::thread::Builder::new()
        .name("silo-drain".into())
        .spawn(move || {
            for ev in rx.iter() {
                match ev {
                    Event::ExecDone(ms) => {
                        tracing::debug!(target = "exec", "exec done in {ms}ms");
                    }
                    other => {
                        tracing::debug!(target = "exec", "{other:?}");
                    }
                }
                stats.ingest_event(ev);
            }
        })
        .expect("drain thread");
}

fn build_engine(
    workers: usize,
    timeout: Duration,
    asn: u32,
    proxies: &[String],
) -> Result<VisitCtx, String> {
    let stats = Arc::new(StatBlock::new());
    let (tx, rx): (EventTx, _) = crossbeam_channel::bounded(8192);
    spawn_event_drain(rx, Arc::clone(&stats));
    for raw in proxies {
        if let Ok(cfg) = flow::proxy_cfg(raw)
            && let Some(geo) = session_state::geo_for_host(cfg.host().as_str())
        {
            let country = core_utils::country_of(geo.tz).unwrap_or("");
            let a = session_state::assess_proxy(asn, country, geo.tz, "en-US");
            tracing::warn!(
                target = "net",
                "proxy {} => {} score={:.2} safe={} issues=0x{:x}",
                cfg.redacted_string(),
                a.kind.as_str(),
                a.score,
                session_state::is_safe_for_signup(&a),
                a.issues
            );
        }
    }
    let bundle = Arc::new(
        Bundle::open(POLYFILL_PATH)
            .unwrap_or_else(|_| Bundle::from_source(Arc::from(POLYFILL_EMBED))),
    );
    let pool = WorkerPool::spawn(workers, bundle, tx, 4096)?;
    let catalog = engine_catalog_with_proxies(proxies)?;
    let engines = Arc::new(EngineSet::build(&catalog).map_err(|e| e.to_string())?);
    let profiles: Vec<Arc<Profile>> = catalog.iter().map(|c| c.profile.clone()).collect();
    Ok(VisitCtx {
        engines,
        pool: Arc::new(pool),
        stats,
        profiles,
        timeout,
        asn,
        overrides: Arc::new(SiteOverrides::from_env()),
        robots: Arc::new(net_client::RobotsCache::new()),
    })
}

fn with_engine<F, Fut>(workers: usize, timeout: Duration, asn: u32, proxies: &[String], f: F)
where
    F: FnOnce(VisitCtx) -> Fut,
    Fut: Future<Output = i32>,
{
    let workers = workers.max(1);
    match build_engine(workers, timeout, asn, proxies) {
        Ok(engine) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let code = rt.block_on(f(engine));
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("engine init failed: {e}");
            std::process::exit(1);
        }
    }
}

fn main() {
    raise_timer_resolution();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    match cli.cmd {
        Cmd::Run {
            url,
            workers,
            timeout_ms,
            asn,
            proxy,
        } => {
            with_engine(
                workers,
                Duration::from_millis(timeout_ms),
                asn,
                &proxy,
                |engine| async move {
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();
                    for u in &url {
                        let _ = writeln!(&mut out, "== {u}");
                        render_visit(&mut out, &flow::visit(&engine, flow::VisitReq { url: u, selectors: &[], proxy: None }).await);
                        let _ = out.flush();
                    }
                    render_footer(&mut out, &engine.stats);
                    0
                },
            );
        }
        Cmd::Serve {
            listen,
            workers,
            timeout_ms,
            asn,
            proxy,
        } => {
            with_engine(
                workers,
                Duration::from_millis(timeout_ms),
                asn,
                &proxy,
                |engine| async move {
                    let state = api::AppState::spawn(api::AppSpawn {
                        workers,
                        monitor: parser_pipeline::VersionMonitor::new(),
                        ctx: engine,
                    });
                    let app = api::router(state);
                    let listener = tokio::net::TcpListener::bind(&listen).await.expect("bind");
                    tracing::info!("listening: http://{}", listen);
                    axum::serve(listener, app).await.expect("server");
                    0
                },
            );
        }
        Cmd::Bench {
            url,
            n,
            concurrency,
            asn,
            proxy,
        } => {
            with_engine(
                2,
                Duration::from_millis(150),
                asn,
                &proxy,
                |engine| async move {
                    let (slot, profile) = profile_for_host_with_overrides(
                        &engine.profiles,
                        &url,
                        engine.asn,
                        &engine.overrides,
                    );
                    let sem = Arc::new(Semaphore::new(concurrency));
                    let start = Instant::now();
                    let url: Arc<str> = Arc::from(url.as_str());
                    let mut handles = Vec::with_capacity(n);
                    for _ in 0..n {
                        let permit = Arc::clone(&sem).acquire_owned().await;
                        let engines = Arc::clone(&engine.engines);
                        let stats = Arc::clone(&engine.stats);
                        let profile = Arc::clone(&profile);
                        let url = Arc::clone(&url);
                        handles.push(tokio::spawn(async move {
                            let t0 = Instant::now();
                            let mut session = Session::new(profile, &url);
                            let r: Result<Fetched, _> =
                                net_client::fetch_page(&engines, slot, &mut session, &url).await;
                            drop(permit);
                            match r {
                                Ok(f) => {
                                    stats.add_fetch(f.bytes_in);
                                    Some(ms(t0, Instant::now()))
                                }
                                Err(_) => None,
                            }
                        }));
                    }
                    let mut durs = Vec::with_capacity(n);
                    for h in handles {
                        if let Ok(Some(ms)) = h.await {
                            durs.push(ms);
                        }
                    }
                    let total = start.elapsed().as_secs_f64();
                    let (p50, p99) = p50p99(&mut durs);
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();
                    let _ = writeln!(
                        &mut out,
                        "n={n} p50={p50}ms p99={p99}ms rps={:.2}",
                        n as f64 / total.max(f64::EPSILON)
                    );
                    render_footer(&mut out, &engine.stats);
                    0
                },
            );
        }
    }
}
