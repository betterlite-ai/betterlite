use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use opentelemetry_sdk::trace::SpanProcessor;

// Workers closing shed destination processors, so cycling a tenant's config cannot spawn threads per request.
pub const DRAIN_WORKERS: usize = 2;

// Shed processors waiting to be closed before the fan-out stops building new ones.
pub const MAX_PENDING_DRAINS: usize = 64;

// How long shutdown waits for spans already being forwarded, so teardown closes no processor under one.
pub const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct DrainState {
    closed: bool,
    // submitted processors whose close has not returned
    backlog: usize,
    live_workers: usize,
    queue: VecDeque<Arc<dyn SpanProcessor>>,
}

#[derive(Debug, Default)]
struct DrainShared {
    state: Mutex<DrainState>,
    wake: Condvar,
}

// litellm.integrations.otel.plumbing.providers._DrainPool
#[derive(Debug)]
pub struct DrainPool {
    shared: Arc<DrainShared>,
    capacity: usize,
    // kept so the pool owns its workers; Drop below tells them to finish and exit
    _workers: Vec<JoinHandle<()>>,
}

impl DrainPool {
    #[must_use]
    pub fn new(workers: usize, capacity: usize) -> Self {
        let shared = Arc::new(DrainShared::default());
        let mut handles = Vec::with_capacity(workers);
        for index in 0..workers {
            let worker_shared = Arc::clone(&shared);
            let spawned = thread::Builder::new()
                .name(format!("otel-destination-drain-{index}"))
                .spawn(move || drain_until_closed(&worker_shared));
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    tracing::error!(%error, "otel: could not start a destination drain worker");
                }
            }
        }
        let live = handles.len();
        let mut state = match shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.live_workers = live;
        drop(state);
        Self {
            shared,
            capacity,
            _workers: handles,
        }
    }

    // litellm.integrations.otel.plumbing.providers._DrainPool.submit
    pub fn submit(&self, processor: Arc<dyn SpanProcessor>) {
        let mut state = match self.shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !state.closed && state.live_workers > 0 {
            state.backlog = state.backlog.saturating_add(1);
            state.queue.push_back(processor);
            self.shared.wake.notify_one();
            return;
        }
        // No worker was ever started, so this thread closes the processor itself.
        let close_inline = !state.closed;
        drop(state);
        if close_inline {
            shutdown_quietly(&processor);
            return;
        }
        // Past close, closing on the span-ending thread would park it on a flush shutdown no longer waits for.
        let straggler = thread::Builder::new()
            .name("otel-destination-drain-straggler".to_owned())
            .spawn(move || shutdown_quietly(&processor));
        if let Err(error) = straggler {
            tracing::error!(%error, "otel: could not start a destination drain straggler");
        }
    }

    // litellm.integrations.otel.plumbing.providers._DrainPool.saturated
    #[must_use]
    pub fn saturated(&self) -> bool {
        let state = match self.shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let backlog = state.backlog;
        drop(state);
        backlog >= self.capacity
    }

    // litellm.integrations.otel.plumbing.providers._DrainPool.close
    pub fn close(&self, timeout: Option<Duration>) {
        let mut state = match self.shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.closed {
            return;
        }
        state.closed = true;
        self.shared.wake.notify_all();
        let Some(limit) = timeout else {
            return;
        };
        let deadline = Instant::now()
            .checked_add(limit)
            .unwrap_or_else(Instant::now);
        while state.live_workers > 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            let waited = self.shared.wake.wait_timeout(state, remaining);
            state = match waited {
                Ok((guard, _timed_out)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        drop(state);
    }
}

impl Drop for DrainPool {
    fn drop(&mut self) {
        let mut state = match self.shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.closed = true;
        self.shared.wake.notify_all();
        drop(state);
    }
}

// litellm.integrations.otel.plumbing.providers._DrainPool._drain_until_closed
fn drain_until_closed(shared: &DrainShared) {
    while let Some(processor) = next_processor(shared) {
        shutdown_quietly(&processor);
        let mut state = match shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.backlog = state.backlog.saturating_sub(1);
        shared.wake.notify_all();
        drop(state);
    }
}

// A closed worker that finds the queue empty retires here and reports itself gone.
fn next_processor(shared: &DrainShared) -> Option<Arc<dyn SpanProcessor>> {
    let mut state = match shared.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    loop {
        if let Some(processor) = state.queue.pop_front() {
            return Some(processor);
        }
        if state.closed {
            state.live_workers = state.live_workers.saturating_sub(1);
            shared.wake.notify_all();
            return None;
        }
        state = match shared.wake.wait(state) {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
    }
}

// litellm.integrations.otel.plumbing.providers._shutdown_quietly
fn shutdown_quietly(processor: &Arc<dyn SpanProcessor>) {
    if let Err(error) = processor.shutdown() {
        tracing::debug!(%error, "otel: closing a destination processor failed");
    }
}

#[path = "drain/tests.rs"]
mod tests;
