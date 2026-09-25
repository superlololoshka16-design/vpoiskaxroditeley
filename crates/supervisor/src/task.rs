use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use sonic_rs::{JsonValueMutTrait as _, JsonValueTrait as _};

use core_utils::ms;

pub type TaskId = u64;

pub const TASK_QUEUE_CAP: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskKind {
    HtmlFetch {
        url: String,
        #[serde(default)]
        proxy: Option<String>,
    },
    Extract {
        url: String,
        selectors: Vec<(String, String)>,
        #[serde(default)]
        proxy: Option<String>,
    },
    FormSubmit {
        url: String,
        method: Option<String>,
        fields: Vec<(String, String)>,
        token_field: Option<String>,
        token: Option<String>,
        #[serde(default)]
        proxy: Option<String>,
    },
    Solve {
        kind: String,
        script: Option<String>,
        payload_b64: Option<String>,
        deadline_ms: Option<u64>,
        profile_id: Option<u64>,
        #[serde(default)]
        proxy: Option<String>,
    },
}

impl TaskKind {
    pub(crate) fn url_ref(&self) -> &str {
        match self {
            TaskKind::HtmlFetch { url, .. }
            | TaskKind::Extract { url, .. }
            | TaskKind::FormSubmit { url, .. } => url,
            TaskKind::Solve { .. } => "",
        }
    }
}

pub(crate) const ST_QUEUED: u8 = 0;
pub(crate) const ST_PROCESSING: u8 = 1;
pub(crate) const ST_READY: u8 = 2;
pub(crate) const ST_FAILED: u8 = 3;

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

pub struct TaskRec {
    pub submitted: Instant,
    pub deadline: Instant,
    pub state: AtomicU8,
    pub result: OnceLock<Arc<sonic_rs::Value>>,
    pub encoded: OnceLock<bytes::Bytes>,
    pub _admission: tokio::sync::OwnedSemaphorePermit,
}

impl TaskRec {
    pub fn new(ttl: Duration, admission: tokio::sync::OwnedSemaphorePermit) -> Self {
        let now = Instant::now();
        Self {
            submitted: now,
            deadline: now.checked_add(ttl).unwrap_or(now),
            state: AtomicU8::new(ST_QUEUED),
            result: OnceLock::new(),
            encoded: OnceLock::new(),
            _admission: admission,
        }
    }

    pub fn alive(&self) -> bool {
        Instant::now() < self.deadline
    }

    fn failed_outcome(&self) -> TaskOutcome {
        let error = self
            .result
            .get()
            .and_then(|v| v["error"].as_str().map(compact_str::CompactString::from))
            .unwrap_or_else(|| compact_str::CompactString::const_new("failed"));
        TaskOutcome::Failed { error }
    }

    pub fn outcome(&self) -> TaskOutcome {
        let now = Instant::now();
        match self.state.load(Ordering::Acquire) {
            ST_QUEUED | ST_PROCESSING => {
                if now >= self.deadline {
                    return TaskOutcome::Failed {
                        error: compact_str::CompactString::const_new("expired"),
                    };
                }
                TaskOutcome::Processing {
                    elapsed_ms: ms(self.submitted, now),
                    ttl_left_ms: ms(now, self.deadline),
                }
            }
            ST_FAILED => self.failed_outcome(),
            _ => match self.result.get() {
                Some(_) if now > self.deadline => TaskOutcome::Failed {
                    error: compact_str::CompactString::const_new("expired"),
                },
                Some(v) => TaskOutcome::Ready {
                    solution: Arc::clone(v),
                },
                None => TaskOutcome::Failed {
                    error: compact_str::CompactString::const_new("no result"),
                },
            },
        }
    }
}

pub fn next_task_id() -> TaskId {
    NEXT_TASK.fetch_add(1, Ordering::Relaxed)
}

pub struct Job {
    pub id: TaskId,
    pub kind: TaskKind,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TaskOutcome {
    Processing { elapsed_ms: u64, ttl_left_ms: u64 },
    Ready { solution: Arc<sonic_rs::Value> },
    Failed { error: compact_str::CompactString },
}

pub fn canonical_method(m: &str) -> Option<&'static str> {
    ["GET", "PUT", "PATCH", "POST"]
        .into_iter()
        .find(|x| m.eq_ignore_ascii_case(x))
}

pub fn canonical_or_get(m: &str) -> &'static str {
    canonical_method(m).unwrap_or("GET")
}

pub fn req_by_method(client: &wreq::Client, method: &str, url: &str) -> wreq::RequestBuilder {
    match method {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        _ => client.get(url),
    }
}

pub type FormPairs<'a> = SmallVec<[(&'a str, &'a str); 24]>;

pub struct FormBuilder<'a> {
    pub body: FormPairs<'a>,
    pub index: std::collections::HashMap<&'a str, usize, core_utils::FxBuild>,
}

impl<'a> FormBuilder<'a> {
    pub fn new() -> Self {
        Self {
            body: FormPairs::new(),
            index: core_utils::fx_map(),
        }
    }

    pub fn upsert(&mut self, k: &'a str, v: &'a str) {
        match self.index.get_mut(k) {
            Some(i) => self.body[*i].1 = v,
            None => {
                self.index.insert(k, self.body.len());
                self.body.push((k, v));
            }
        }
    }

    pub fn done(self) -> FormPairs<'a> {
        self.body
    }
}

pub fn vset(v: &mut sonic_rs::Value, key: &str, val: sonic_rs::Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.insert(key, val);
    }
}

pub fn vstr(x: &compact_str::CompactString) -> sonic_rs::Value {
    sonic_rs::Value::from(x.as_str())
}

pub fn stamp_outcome(
    v: &mut sonic_rs::Value,
    token: Option<&compact_str::CompactString>,
    nav: Option<&compact_str::CompactString>,
) {
    if let Some(tok) = token {
        vset(v, "solvedToken", vstr(tok));
    }
    if let Some(nav) = nav {
        vset(v, "navigation", vstr(nav));
    }
}
