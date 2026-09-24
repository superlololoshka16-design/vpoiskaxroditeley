pub mod stats {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use runtime_exec::Event;

    #[repr(align(64))]
    struct HotLine {
        fetches: AtomicU64,
        bytes_in: AtomicU64,
        scripts: AtomicU64,
        api_touches: AtomicU64,
    }

    #[repr(align(64))]
    struct ColdLine {
        cache_hits: AtomicU64,
        cache_misses: AtomicU64,
        compiles: AtomicU64,
        timeouts: AtomicU64,
        errors: AtomicU64,
        backpressure: AtomicU64,
        wasm_runs: AtomicU64,
    }

    #[repr(align(64))]
    pub struct StatBlock {
        pub started_at: std::time::Instant,
        hot: HotLine,
        cold: ColdLine,
    }

    impl StatBlock {
        pub fn new() -> Self {
            Self {
                started_at: std::time::Instant::now(),
                hot: HotLine {
                    fetches: AtomicU64::new(0),
                    bytes_in: AtomicU64::new(0),
                    scripts: AtomicU64::new(0),
                    api_touches: AtomicU64::new(0),
                },
                cold: ColdLine {
                    cache_hits: AtomicU64::new(0),
                    cache_misses: AtomicU64::new(0),
                    compiles: AtomicU64::new(0),
                    timeouts: AtomicU64::new(0),
                    errors: AtomicU64::new(0),
                    backpressure: AtomicU64::new(0),
                    wasm_runs: AtomicU64::new(0),
                },
            }
        }

        pub fn ingest_event(&self, ev: Event) {
            match ev {
                Event::CacheHit => {
                    self.cold.cache_hits.fetch_add(1, Ordering::Relaxed);
                }
                Event::ExecDone(_) => {}
                Event::CacheMiss => {
                    self.cold.cache_misses.fetch_add(1, Ordering::Relaxed);
                }
                Event::Compile => {
                    self.cold.compiles.fetch_add(1, Ordering::Relaxed);
                }
                Event::Timeout => {
                    self.cold.timeouts.fetch_add(1, Ordering::Relaxed);
                }
                Event::Oom
                | Event::ExecFail
                | Event::ParseFail
                | Event::NoResult
                | Event::WasmFail => {
                    self.cold.errors.fetch_add(1, Ordering::Relaxed);
                }
                Event::Backpressure => {
                    self.cold.backpressure.fetch_add(1, Ordering::Relaxed);
                }
                Event::WasmRun => {
                    self.cold.wasm_runs.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        pub fn add_fetch(&self, bytes: u64) {
            self.hot.fetches.fetch_add(1, Ordering::Relaxed);
            self.hot.bytes_in.fetch_add(bytes, Ordering::Relaxed);
        }

        pub fn fetches(&self) -> u64 {
            self.hot.fetches.load(Ordering::Relaxed)
        }
        pub fn bytes(&self) -> u64 {
            self.hot.bytes_in.load(Ordering::Relaxed)
        }
        pub fn scripts(&self) -> u64 {
            self.hot.scripts.load(Ordering::Relaxed)
        }

        pub fn add_script(&self) {
            self.hot.scripts.fetch_add(1, Ordering::Relaxed);
        }

        pub fn add_touches(&self, n: u64) {
            self.hot.api_touches.fetch_add(n, Ordering::Relaxed);
        }

        pub fn touches(&self) -> u64 {
            self.hot.api_touches.load(Ordering::Relaxed)
        }

        pub fn uptime_secs(&self) -> u64 {
            self.started_at.elapsed().as_secs()
        }

        pub fn render(&self, out: &mut std::io::StdoutLock<'_>) {
            let fetches = self.hot.fetches.load(Ordering::Relaxed);
            let bytes_in = self.hot.bytes_in.load(Ordering::Relaxed);
            let scripts = self.hot.scripts.load(Ordering::Relaxed);
            let hits = self.cold.cache_hits.load(Ordering::Relaxed);
            let misses = self.cold.cache_misses.load(Ordering::Relaxed);
            let compiles = self.cold.compiles.load(Ordering::Relaxed);
            let timeouts = self.cold.timeouts.load(Ordering::Relaxed);
            let errors = self.cold.errors.load(Ordering::Relaxed);
            let backpressure = self.cold.backpressure.load(Ordering::Relaxed);
            let wasm = self.cold.wasm_runs.load(Ordering::Relaxed);
            let api_touches = self.hot.api_touches.load(Ordering::Relaxed);
            let uptime = self.uptime_secs();
            let _ = writeln!(
                out,
                "fetches={fetches} bytes_in={bytes_in} scripts={scripts} cache_hits={hits} cache_misses={misses} compiles={compiles} timeouts={timeouts} errors={errors} backpressure={backpressure} wasm={wasm} api_touches={api_touches} uptime={uptime}s"
            );
        }
    }

    pub fn render_rss(out: &mut std::io::StdoutLock<'_>) {
        let mut buf = String::new();
        if std::fs::File::open("/proc/self/status")
            .and_then(|mut f| std::io::Read::read_to_string(&mut f, &mut buf))
            .is_err()
        {
            let _ = writeln!(out, "rss=unavailable");
            return;
        }
        let pick = |key: &str| {
            buf.lines()
                .find_map(|l| l.strip_prefix(key).map(str::trim))
                .unwrap_or("n/a")
        };
        let _ = writeln!(out, "rss={} hwm={}", pick("VmRSS:"), pick("VmHWM:"));
    }

    pub fn p50p99(durs: &mut [u64]) -> (u64, u64) {
        if durs.is_empty() {
            return (0, 0);
        }
        durs.sort_unstable();
        let p50 = durs[(durs.len() - 1) / 2];
        let p99 = durs[((durs.len() - 1) * 99) / 100];
        (p50, p99)
    }

    pub type StatsRef = Arc<StatBlock>;
}

pub mod api;
pub mod bridge;
pub mod fleet;
pub mod flow;
pub mod site_override;
pub mod task;
pub mod watch;

pub fn ms(a: std::time::Instant, b: std::time::Instant) -> u64 {
    b.saturating_duration_since(a).as_millis() as u64
}

pub fn site_key(host: &str) -> u64 {
    core_utils::xxh3::hash(core_utils::host_of(host).as_bytes())
}

pub fn catalog_slot_of(host: &str, catalog_len: usize) -> usize {
    (site_key(host) as usize) % catalog_len.max(1)
}
