use std::sync::Arc;

use net_client::{EngineSet, set_cookie_lines, with_cookie};
use runtime_exec::{FetchJob, FetchReply, HeaderList, install as install_fetch_bridge};

use super::task::{canonical_or_get, req_by_method};

pub async fn fetch_reply_of(resp: wreq::Response) -> FetchReply {
    let status = resp.status().as_u16();
    let set_cookie = set_cookie_lines(&resp);
    let headers: HeaderList = resp
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|v| {
                (
                    compact_str::CompactString::new(name.as_str()),
                    compact_str::CompactString::new(v),
                )
            })
        })
        .take(32)
        .collect();
    let body = resp.bytes().await.unwrap_or_default();
    FetchReply {
        status,
        headers,
        set_cookie,
        body,
    }
}

pub fn fetch_reply_err() -> FetchReply {
    FetchReply {
        status: 0,
        headers: smallvec::SmallVec::new(),
        set_cookie: smallvec::SmallVec::new(),
        body: bytes::Bytes::new(),
    }
}

pub fn spawn_fetch_daemon(engines: Arc<EngineSet>) {
    let (tx, rx) = crossbeam_channel::bounded::<FetchJob>(512);
    if !install_fetch_bridge(tx) {
        return;
    }

    let _ = std::thread::Builder::new()
        .name("silo-fetch".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
            else {
                return;
            };
            for job in rx.iter() {
                let engines = engines.clone();
                let job = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || job));
                let Ok(job) = job else {
                    continue;
                };
                rt.spawn(async move {
                    if net_client::guard_url(job.url.as_str()).is_err() {
                        let _ = job.reply.send(fetch_reply_err());
                        return;
                    }
                    let client = engines.client_for(job.net_slot);
                    let method = canonical_or_get(job.method.as_str());
                    let mut req = req_by_method(client, method, job.url.as_str());
                    for (k, v) in &job.headers {
                        if let (Ok(name), Ok(value)) = (
                            wreq::header::HeaderName::from_bytes(k.as_bytes()),
                            wreq::header::HeaderValue::from_str(v.as_str()),
                        ) {
                            req = req.header(name, value);
                        }
                    }
                    if !job.cookie.is_empty() {
                        req = with_cookie(req, job.cookie.as_str());
                    }
                    if let Some(body) = job.body {
                        req = req.body(body);
                    }
                    let reply = match req.send().await {
                        Ok(resp) => fetch_reply_of(resp).await,
                        Err(_) => fetch_reply_err(),
                    };
                    let _ = job.reply.send(reply);
                });
            }
        });
}
