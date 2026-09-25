use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use smallvec::SmallVec;

use tokio::sync::{Semaphore, mpsc};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use challenge_solver::cache::solve_with_cache as slider_cache_solve;
use challenge_solver::decode as envelope;
use challenge_solver::slider::SliderExchange;
use net_client::{Fetched, parse_body};
use parser_pipeline::validate_selectors;
use scc::HashMap;
use serde::{Deserialize, Serialize};
use session_state::Profile;
use sonic_rs::JsonValueMutTrait as _;

use crate::bridge::spawn_fetch_daemon;
use crate::fleet::{Fleet, FleetMsg, fleet_daemon};
use crate::flow::{
    self, AnubisFlow, VisitCtx, anubis_answer_value, exec_script, profile_for_task, run_challenge,
};
use core_utils::ms;
use crate::task::{
    Job, ST_FAILED, ST_PROCESSING, ST_READY, TASK_QUEUE_CAP, TaskId, TaskKind, TaskOutcome,
    TaskRec, canonical_method, next_task_id, req_by_method, stamp_outcome, vset, vstr,
};
use crate::watch::{WatchRec, note_build_change, watch_daemon, watch_put, watch_rec};

const CHALLENGE_LOCAL: &str = "https://challenge.local/";
const FLEET_CHAN_CAP: usize = 1024;

pub struct AppState {
    ctx: VisitCtx,
    registry: Arc<HashMap<TaskId, Arc<TaskRec>>>,
    tx: mpsc::Sender<Job>,
    sem: Arc<Semaphore>,
    admission: Arc<Semaphore>,
    monitor: Arc<parser_pipeline::VersionMonitor>,
    fleet_tx: mpsc::Sender<FleetMsg>,
    slider_cache: Arc<challenge_solver::cache::AnswerCache>,
    watched: Arc<HashMap<compact_str::CompactString, WatchRec>>,
    monitor_alerts: Arc<AtomicU64>,
}

pub struct AppSpawn {
    pub workers: usize,
    pub monitor: parser_pipeline::VersionMonitor,
    pub ctx: VisitCtx,
}

impl AppState {
    pub fn spawn(cfg: AppSpawn) -> Arc<Self> {
        let AppSpawn {
            workers,
            monitor,
            ctx,
        } = cfg;
        let registry = Arc::new(HashMap::new());
        let (tx, mut rx) = mpsc::channel::<Job>(TASK_QUEUE_CAP);
        let sem = Arc::new(Semaphore::new(workers.max(1)));
        let (fleet_tx, fleet_rx) = mpsc::channel::<FleetMsg>(FLEET_CHAN_CAP);
        tokio::spawn(fleet_daemon(
            Fleet::new(ctx.engines.clone()),
            fleet_rx,
            ctx.stats.clone(),
        ));
        spawn_fetch_daemon(ctx.engines.clone());
        let state = Arc::new(Self {
            ctx,
            registry,
            tx,
            sem,
            admission: Arc::new(Semaphore::new(TASK_QUEUE_CAP)),
            monitor: Arc::new(monitor),
            fleet_tx,
            slider_cache: Arc::new(challenge_solver::cache::AnswerCache::default()),
            watched: Arc::new(HashMap::new()),
            monitor_alerts: Arc::new(AtomicU64::new(0)),
        });

        let st = state.clone();
        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                let Ok(permit) = st.sem.clone().acquire_owned().await else {
                    break;
                };
                let st = st.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let Some(rec) = st.registry.read_sync(&job.id, |_, r| Arc::clone(r)) else {
                        return;
                    };
                    if !rec.alive() {
                        return;
                    }
                    rec.state.store(ST_PROCESSING, Ordering::Release);
                    let deadline = tokio::time::Instant::from_std(rec.deadline);
                    let out = match tokio::time::timeout_at(deadline, execute(&st, job.kind)).await
                    {
                        Ok(result) => result,
                        Err(_) => Err("deadline exceeded".into()),
                    };
                    match out {
                        Ok(v) => {
                            let _ = rec.result.set(Arc::new(v));
                            rec.state.store(ST_READY, Ordering::Release);
                        }
                        Err(e) => {
                            let _ = rec.result.set(Arc::new(sonic_rs::json!({ "error": e })));
                            rec.state.store(ST_FAILED, Ordering::Release);
                        }
                    }
                });
            }
        });

        let reg = state.registry.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                reg.retain_sync(|_, r| r.alive());
            }
        });

        tokio::spawn(watch_daemon(
            state.ctx.clone(),
            state.watched.clone(),
            state.monitor.clone(),
            state.monitor_alerts.clone(),
        ));

        state
    }

    pub async fn create_task(&self, kind: TaskKind) -> Result<TaskId, String> {
        let url = kind.url_ref();
        if !url.is_empty() {
            let scheme = url.split(':').next().unwrap_or("");
            if !core_utils::url::scheme_is_http(scheme) {
                return Err("bad url".into());
            }
        }
        if let TaskKind::Extract { selectors, .. } = &kind {
            validate_selectors(selectors)?;
        }
        let admission = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| "queue full".to_owned())?;
        let id = next_task_id();
        let rec = Arc::new(TaskRec::new(self.ctx.timeout, admission));
        if self.registry.insert_sync(id, rec).is_err() {
            return Err("task id collision".into());
        }
        if let Err(error) = self.tx.try_send(Job { id, kind }) {
            let _ = self.registry.remove_sync(&id);
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => "queue full",
                mpsc::error::TrySendError::Closed(_) => "workers down",
            }
            .into());
        }
        Ok(id)
    }

    pub fn task_outcome(&self, id: TaskId) -> TaskOutcome {
        match self.registry.read_sync(&id, |_, r| r.outcome()) {
            Some(o) => o,
            None => TaskOutcome::Failed {
                error: compact_str::CompactString::const_new("not found"),
            },
        }
    }

    pub fn stats(&self) -> sonic_rs::Value {
        sonic_rs::json!({
            "tasksInRegistry": self.registry.len(),
            "fetches": self.ctx.stats.fetches(),
            "bytesIn": self.ctx.stats.bytes(),
            "scripts": self.ctx.stats.scripts(),
            "apiTouches": self.ctx.stats.touches(),
            "uptimeSecs": self.ctx.stats.uptime_secs(),
            "monitorAlerts": self.monitor_alerts.load(Ordering::Relaxed),
            "sliderCacheEntries": self.slider_cache.len(),
            "watchedChallengeBuilds": self.watched.len(),
        })
    }
}

async fn execute(st: &AppState, kind: TaskKind) -> Result<sonic_rs::Value, String> {
    match kind {
        TaskKind::HtmlFetch { url, proxy } => fetch_json(st, &url, &[], proxy.as_deref()).await,
        TaskKind::Extract {
            url,
            selectors,
            proxy,
        } => fetch_json(st, &url, &selectors, proxy.as_deref()).await,
        TaskKind::FormSubmit {
            url,
            method,
            fields,
            token_field,
            token,
            proxy,
        } => {
            let proxy_ref: Option<&str> = proxy.as_deref();
            submit_json(SubmitCtx {
                st,
                url: &url,
                method: &method,
                fields: &fields,
                token_field: &token_field,
                token: &token,
                proxy: proxy_ref,
            })
            .await
        }
        TaskKind::Solve {
            kind,
            script,
            payload_b64,
            deadline_ms,
            profile_id,
            proxy,
        } => {
            let proxy_ref: Option<&str> = proxy.as_deref();
            solve_json(
                SolveCtx {
                    st,
                    script,
                    payload_b64,
                    deadline_ms,
                    profile_id,
                    proxy: proxy_ref,
                },
                &kind,
            )
            .await
        }
    }
}

fn page_json(f: &Fetched) -> sonic_rs::Value {
    sonic_rs::json!({
        "finalUrl": f.uri.as_str(),
        "statusCode": f.status,
        "htmlLen": f.bytes_in,
        "title": f.page.title,
        "metaDescription": f.page.meta_description,
        "forms": f.page.forms.iter().map(|fm| sonic_rs::json!({
            "action": fm.action,
            "method": fm.method,
            "fields": fm.fields.iter().map(|fd| {
                sonic_rs::json!({ "name": fd.name, "value": fd.value, "kind": fd.kind.as_str() })
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "tokens": f.page.tokens.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
        "challengeMarkers": f.page.challenge_markers.iter().map(|u| u.as_str()).collect::<Vec<_>>(),
        "captchaFamily": f.page.captcha_family.map(|idx| {
            parser_pipeline::CaptchaFamily::from_idx(idx).as_str()
        }),
        "captchaSitekey": f.page.captcha_sitekey,
        "extracted": f.page.extracted,
        "elapsedMs": f.elapsed_ms,
        "challengeVendor": net_client::vendor_label(f.challenge_vendor),
    })
}

async fn fetch_json(
    st: &AppState,
    url: &str,
    selectors: &[(String, String)],
    proxy: Option<&str>,
) -> Result<sonic_rs::Value, String> {
    let out = flow::visit(&st.ctx, flow::VisitReq { url, selectors, proxy }).await;
    if let Some(e) = out.error {
        return Err(e);
    }
    let last = out.hops.last().ok_or("visit produced no hops")?;
    let mut v = page_json(&last.fetched);
    if let Some(anubis) = &last.anubis {
        vset(&mut v, "anubis", anubis_json(&last.fetched, anubis));
    }
    stamp_outcome(&mut v, last.token.as_ref(), last.nav.as_ref());
    if let Some(prev) = out.hops[..out.hops.len() - 1]
        .iter()
        .find_map(|h| h.token.as_ref())
    {
        vset(&mut v, "prevToken", vstr(prev));
    }
    if !last.anubis_terminal() {
        if let Some(route) = last.fetched.page.telemetry_route.clone() {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let mut scoped = session_state::CookieJar::new();
            scoped.copy_matching(&last.session.jar, last.fetched.uri.as_str());
            let _ = st
                .fleet_tx
                .send(FleetMsg::Attach {
                    profile: Arc::clone(&last.session.profile),
                    origin: compact_str::CompactString::from(last.fetched.uri.as_str()),
                    cookies: scoped,
                    route,
                    engine_slot: last.slot,
                    weight: 8,
                    reply: reply_tx,
                })
                .await;
            let tab = reply_rx.await.unwrap_or(u32::MAX);
            vset(
                &mut v,
                "telemetry",
                sonic_rs::json!({ "attached": tab != u32::MAX, "tabId": tab }),
            );
        }
        if let (Some(url), Some(script)) = (
            &last.fetched.page.challenge_script_url,
            &last.fetched.page.challenge,
        ) {
            note_build_change(&st.monitor, url.as_str(), script, &st.monitor_alerts);
            watch_put(
                &st.watched,
                url.as_str(),
                watch_rec(false, bytes::Bytes::clone(script)),
            );
        } else if let Some(script) = &last.fetched.page.challenge {
            watch_put(
                &st.watched,
                last.url.as_str(),
                watch_rec(true, bytes::Bytes::clone(script)),
            );
        }
    }
    Ok(v)
}

fn anubis_json(f: &Fetched, anub: &Result<AnubisFlow, String>) -> sonic_rs::Value {
    let flow = match anub {
        Ok(fl) => fl,
        Err(e) => return sonic_rs::json!({ "solved": false, "error": e }),
    };
    let mut out = sonic_rs::json!({
        "solved": true,
        "answer": &flow.answer,
        "version": f.page.anubis_version,
    });
    match &flow.pass {
        Ok(r) => {
            vset(
                &mut out,
                "authCookie",
                sonic_rs::json!(r.auth_cookie.is_some()),
            );
            vset(&mut out, "passStatus", sonic_rs::json!(r.status));
            vset(&mut out, "hops", sonic_rs::json!(r.hops));
        }
        Err(e) => {
            vset(&mut out, "passError", sonic_rs::json!(e));
        }
    }
    match &flow.refetch {
        Ok(f2) => {
            let passed = flow.passed();
            vset(&mut out, "passed", sonic_rs::json!(passed));
            if passed {
                vset(&mut out, "finalUrl", sonic_rs::json!(f2.uri.as_str()));
                vset(&mut out, "title", sonic_rs::json!(f2.page.title));
            }
        }
        Err(e) => {
            vset(&mut out, "refetchError", sonic_rs::json!(e));
        }
    }
    out
}

struct SubmitCtx<'a> {
    st: &'a AppState,
    url: &'a str,
    method: &'a Option<String>,
    fields: &'a [(String, String)],
    token_field: &'a Option<String>,
    token: &'a Option<String>,
    proxy: Option<&'a str>,
}

async fn submit_json(cx: SubmitCtx<'_>) -> Result<sonic_rs::Value, String> {
    let (url, method, fields, token_field, token, proxy) = (
        cx.url,
        cx.method,
        cx.fields,
        cx.token_field,
        cx.token,
        cx.proxy,
    );
    let st = cx.st;
    let ctx = &st.ctx;
    let (slot, mut session) = flow::session_for(ctx, url, proxy)?;
    let f = flow::fetch_counted(ctx, slot, &mut session, url, &[]).await?;
    let mut fb = crate::task::FormBuilder::new();
    for fm in &f.page.forms {
        for fd in &fm.fields {
            fb.upsert(fd.name.as_str(), fd.value.as_deref().unwrap_or(""));
        }
    }
    for (k, v) in fields {
        fb.upsert(k.as_str(), v.as_str());
    }
    if let (Some(tf), Some(tv)) = (token_field, token) {
        fb.upsert(tf.as_str(), tv.as_str());
    }
    let body = fb.done();
    let m_str = method
        .as_deref()
        .or_else(|| f.page.forms.first().map(|fm| fm.method.as_str()))
        .unwrap_or("POST");
    let m = canonical_method(m_str).ok_or("bad method")?;
    let client = ctx.engines.client_for(slot);
    let submit_start = Instant::now();
    let resp = req_by_method(client, m, url)
        .form(body.as_slice())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body_bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let elapsed = ms(submit_start, Instant::now());
    let page = parse_body(&body_bytes).map_err(|e| e.to_string())?;
    let mut v = sonic_rs::json!({ "statusCode": status, "htmlLen": body_bytes.len() });
    if let Some(r) = run_challenge(
        ctx,
        &mut session,
        &Fetched {
            status,
            uri: compact_str::CompactString::from(url),
            page: Arc::new(page),
            bytes_in: body_bytes.len() as u64,
            elapsed_ms: elapsed,
            challenge_vendor: net_client::VENDOR_NONE,
        },
        slot,
    )
    .await
    {
        stamp_outcome(&mut v, r.token.as_ref(), r.nav.as_ref());
    }
    Ok(v)
}

fn script_raw_of(
    script: Option<String>,
    payload_b64: Option<String>,
    what: &str,
) -> Result<Bytes, String> {
    match (script, payload_b64) {
        (Some(s), _) => Ok(Bytes::from(s.into_bytes())),
        (None, Some(b64)) => core_utils::B64_STANDARD
            .decode(b64.trim_ascii().as_bytes())
            .map(Bytes::from)
            .map_err(|e| format!("payload_b64: {e}")),
        (None, None) => Err(format!("{what} unavailable")),
    }
}


struct SolveCtx<'a> {
    st: &'a AppState,
    script: Option<String>,
    payload_b64: Option<String>,
    deadline_ms: Option<u64>,
    profile_id: Option<u64>,
    proxy: Option<&'a str>,
}

async fn solve_json(cx: SolveCtx<'_>, kind: &str) -> Result<sonic_rs::Value, String> {
    let ctx = &cx.st.ctx;
    let (net_slot, profile) = profile_for_task(ctx, cx.profile_id, cx.proxy)?;
    let what = if kind == "anubis" {
        "anubis: challenge json"
    } else if kind == "slider" || kind == "captcha" {
        "slider: envelope"
    } else {
        return solve_js(cx, kind, net_slot, profile).await;
    };
    let raw = script_raw_of(cx.script, cx.payload_b64, what)?;
    if kind == "anubis" {
        let (ch, sol) = flow::solve_anubis(flow::AnubisReq { ctx, raw, href: CHALLENGE_LOCAL, profile: &profile, slot: 0 }).await?;
        return Ok(sonic_rs::json!({
            "solved": true,
            "algorithm": ch.algorithm.name(),
            "difficulty": ch.difficulty,
            "answer": anubis_answer_value(&sol),
        }));
    }
    solve_slider(cx.st, raw).await
}

async fn solve_js(
    cx: SolveCtx<'_>,
    kind: &str,
    net_slot: usize,
    profile: Arc<Profile>,
) -> Result<sonic_rs::Value, String> {
    let ctx = &cx.st.ctx;
    let what = format!("bytecode for {kind}");
    let raw = script_raw_of(cx.script, cx.payload_b64, what.as_str())?;
    let timeout = cx
        .deadline_ms
        .map(Duration::from_millis)
        .unwrap_or(ctx.timeout);
    let outcome = exec_script(&ctx.pool, &profile, CHALLENGE_LOCAL, raw, timeout, net_slot).await;
    match outcome.token {
        Some(t) => {
            let path = match outcome.path {
                runtime_exec::ExecPath::RawHit => "rawHit",
                runtime_exec::ExecPath::NormHit => "normHit",
                runtime_exec::ExecPath::Compile => "compile",
                runtime_exec::ExecPath::Wasm => "wasm",
            };
            Ok(sonic_rs::json!({ "solvedToken": t, "path": path }))
        }
        None => Err(outcome
            .err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "solve failed".into())),
    }
}

async fn solve_slider(
    st: &AppState,
    raw: Bytes,
) -> Result<sonic_rs::Value, String> {
    thread_local! {
        static SCRATCH: RefCell<envelope::Scratch> = RefCell::new(envelope::Scratch::default());
    }
    let (meta, outcome) = tokio::task::block_in_place(|| {
        SCRATCH.with(|s| {
            let mut scratch = s.borrow_mut();
            let (meta, payload) =
                envelope::decode(&raw, &mut scratch).map_err(|e| format!("decode: {e:?}"))?;
            let ex =
                SliderExchange::parse(payload).map_err(|e| format!("slider exchange: {e:?}"))?;
            let outcome = slider_cache_solve(&st.slider_cache, &ex, payload)
                .map_err(|e| format!("slider solve: {e:?}"))?;
            Ok::<_, String>((meta, outcome))
        })
    })?;
    let (cache_hit, ssd) = match outcome.kind {
        challenge_solver::cache::OutcomeKind::CacheHit => (true, 0),
        challenge_solver::cache::OutcomeKind::Computed { ssd } => (false, ssd),
    };
    Ok(sonic_rs::json!({
        "solved": true,
        "x": outcome.x,
        "y": outcome.y,
        "cacheHit": cache_hit,
        "ssd": ssd,
        "alg": meta.kind.label(),
    }))
}
fn json_raw(status: StatusCode, body: Bytes) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn json_ok<T: serde::Serialize>(t: &T) -> Response {
    match sonic_rs::to_vec(t) {
        Ok(body) => json_raw(StatusCode::OK, Bytes::from(body)),
        Err(_) => json_err(StatusCode::INTERNAL_SERVER_ERROR, "encode failed"),
    }
}

fn json_err(status: StatusCode, msg: &str) -> Response {
    let body = sonic_rs::to_vec(&sonic_rs::json!({ "error": msg })).unwrap_or_default();
    json_raw(status, Bytes::from(body))
}

fn json_body<T: serde::de::DeserializeOwned>(bytes: &Bytes) -> Result<T, Response> {
    sonic_rs::from_slice(bytes.as_ref())
        .map_err(|e| json_err(StatusCode::BAD_REQUEST, &format!("bad json: {e}")))
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "my-engine: alive" }))
        .route("/stats", get(stats_handler))
        .route("/createTask", post(create_task))
        .route("/getTaskResult", post(get_result))
        .with_state(state)
}

async fn stats_handler(State(s): State<Arc<AppState>>) -> Response {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let _ = s.fleet_tx.send(FleetMsg::Stats { reply: reply_tx }).await;
    let mut v = s.stats();
    if let Some(fs) = reply_rx.await.ok()
        && let Some(dst) = v.as_object_mut()
    {
        dst.insert("liveTabs", sonic_rs::json!(fs.live_tabs));
        dst.insert("events", sonic_rs::json!(fs.events));
        dst.insert("batches", sonic_rs::json!(fs.batches));
    }
    json_ok(&v)
}

async fn create_task(State(s): State<Arc<AppState>>, body: Bytes) -> Response {
    let req: CreateTaskReq = match json_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match s.create_task(req.task).await {
        Ok(id) => json_ok(&CreateTaskResp { task_id: id }),
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn get_result(State(s): State<Arc<AppState>>, body: Bytes) -> Response {
    let req: GetResultReq = match json_body(&body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match s.task_outcome(req.task_id) {
        TaskOutcome::Ready { solution } => {
            let body = s.registry.read_sync(&req.task_id, |_, rec| {
                rec.encoded
                    .get_or_init(|| {
                        sonic_rs::to_vec(&TaskOutcome::Ready {
                            solution: Arc::clone(&solution),
                        })
                        .map(bytes::Bytes::from)
                        .unwrap_or_default()
                    })
                    .clone()
            });
            match body {
                Some(b) if !b.is_empty() => json_raw(StatusCode::OK, b),
                _ => json_err(StatusCode::INTERNAL_SERVER_ERROR, "encode failed"),
            }
        }
        TaskOutcome::Failed { error } => {
            let body = s.registry.read_sync(&req.task_id, |_, rec| {
                rec.encoded
                    .get_or_init(|| {
                        sonic_rs::to_vec(&TaskOutcome::Failed {
                            error: error.clone(),
                        })
                        .map(bytes::Bytes::from)
                        .unwrap_or_default()
                    })
                    .clone()
            });
            match body {
                Some(b) if !b.is_empty() => json_raw(StatusCode::OK, b),
                None => {
                    let v = sonic_rs::json!({ "status": "failed", "error": error.as_str() });
                    match sonic_rs::to_vec(&v) {
                        Ok(b) => json_raw(StatusCode::OK, bytes::Bytes::from(b)),
                        Err(_) => json_err(StatusCode::INTERNAL_SERVER_ERROR, "encode failed"),
                    }
                }
                _ => json_err(StatusCode::INTERNAL_SERVER_ERROR, "encode failed"),
            }
        }
        other => json_ok(&other),
    }
}

#[derive(Deserialize)]
pub struct CreateTaskReq {
    pub task: TaskKind,
}

#[derive(Serialize)]
pub struct CreateTaskResp {
    pub task_id: TaskId,
}

#[derive(Deserialize)]
pub struct GetResultReq {
    pub task_id: TaskId,
}
