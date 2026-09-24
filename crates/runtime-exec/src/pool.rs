use crate::cache::{BYTES_PER_ENTRY, Budget, NormCache};
use crate::task::{CancelOnDrop, EventTx, ExecControl, ExecError, ExecOutcome, ExecReq, ExecTask};
use crate::worker::Bundle;
use std::future::Future;
use std::sync::Arc;

pub struct WorkerPool {
    tx: crate::task::ExecQueueTx,
    events: EventTx,
    script_budget: Arc<Budget>,
}

const QUEUE_CAP: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub struct WorkerPoolLimits {
    pub queue_capacity: usize,
    pub script_bytes: u32,
    pub cache_entries: usize,
    pub cache_bytes: usize,
}

impl Default for WorkerPoolLimits {
    fn default() -> Self {
        Self {
            queue_capacity: QUEUE_CAP,
            script_bytes: 32 * 1024 * 1024,
            cache_entries: 4096,
            cache_bytes: 16 * 1024 * 1024,
        }
    }
}

impl WorkerPool {
    pub fn spawn(
        workers: usize,
        bundle: Arc<Bundle>,
        events: EventTx,
        cache_cap: usize,
    ) -> Result<Self, String> {
        Self::spawn_with_limits(
            workers,
            bundle,
            events,
            WorkerPoolLimits {
                cache_entries: cache_cap,
                cache_bytes: cache_cap.saturating_mul(BYTES_PER_ENTRY),
                ..WorkerPoolLimits::default()
            },
        )
    }

    pub fn spawn_with_limits(
        workers: usize,
        bundle: Arc<Bundle>,
        events: EventTx,
        limits: WorkerPoolLimits,
    ) -> Result<Self, String> {
        if workers == 0 {
            return Err("at least one worker required".into());
        }
        if limits.queue_capacity == 0 || limits.script_bytes == 0 {
            return Err("queue and script budgets must be positive".into());
        }
        let cache = Arc::new(NormCache::with_budget(
            limits.cache_entries,
            limits.cache_bytes,
        ));
        crate::intl::warmup();
        let (tx, rx) = crate::task::exec_channel(limits.queue_capacity);
        for i in 0..workers {
            let rx = rx.clone();
            let bundle = Arc::clone(&bundle);
            let events = events.clone();
            let cache = Arc::clone(&cache);
            std::thread::Builder::new()
                .name("silo-js".into())
                .spawn(move || crate::worker::worker_main(i, rx, bundle, events, cache))
                .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            tx,
            events,
            script_budget: Budget::new(u64::MAX as usize, limits.script_bytes as usize),
        })
    }

    fn backpressure(&self) -> ExecOutcome {
        let _ = self.events.try_send(crate::task::Event::Backpressure);
        ExecOutcome::failed(ExecError::Backpressure)
    }

    pub fn exec(&self, req: ExecReq) -> impl Future<Output = ExecOutcome> + Send + use<> {
        let control = ExecControl::new(req.timeout);
        let deadline = tokio::time::Instant::from_std(control.deadline);
        let cancel = CancelOnDrop(Arc::clone(&control));
        let (r_tx, r_rx) = tokio::sync::oneshot::channel();
        let queued = if control.stopped() {
            Err(ExecOutcome::failed(ExecError::Timeout))
        } else {
            match self.script_budget.reserve(1, req.script.len() as u64) {
                Some(script_charge) => {
                    let priority = ExecTask::priority_of(req.kind);
                    match self.tx.try_send(ExecTask {
                        priority,
                        req,
                        reply: r_tx,
                        control,
                        _script_charge: script_charge,
                    }) {
                        Ok(()) => Ok(r_rx),
                        Err(crate::task::TrySendError::Full) => Err(self.backpressure()),
                        Err(crate::task::TrySendError::Disconnected) => {
                            Err(ExecOutcome::failed(ExecError::Shutdown))
                        }
                    }
                }
                None => Err(self.backpressure()),
            }
        };
        async move {
            let _cancel = cancel;
            match queued {
                Err(result) => result,
                Ok(reply) => match tokio::time::timeout_at(deadline, reply).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => ExecOutcome::failed(ExecError::Shutdown),
                    Err(_) => ExecOutcome::failed(ExecError::Timeout),
                },
            }
        }
    }
}
