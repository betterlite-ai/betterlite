use opentelemetry_sdk::trace::SpanProcessor;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
pub const DRAIN_WORKERS: usize = 2;
pub const MAX_PENDING_DRAINS: usize = 64;
pub const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);
#[derive(Debug, Default)]
struct DrainState {
    closed: bool,
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
    // kept so the pool owns its workers; Drop tells them to finish and exit
    _workers: Vec<JoinHandle<()>>,
}

impl DrainPool {
    #[must_use]
    pub fn new(workers: usize, capacity: usize) -> Self {
        let shared = Arc::new(DrainShared::default());
        let mut handles = Vec::with_capacity(workers);
        for index in 0..workers {
            let worker = Arc::clone(&shared);
            let spawned = thread::Builder::new()
                .name(format!("otel-destination-drain-{index}"))
                .spawn(move || drain_until_closed(&worker));
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    tracing::error!(%error, "otel: could not start a destination drain worker");
                }
            }
        }
        shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live_workers = handles.len();
        Self {
            shared,
            capacity,
            _workers: handles,
        }
    }

    pub fn submit(&self, processor: Arc<dyn SpanProcessor>) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !state.closed && state.live_workers > 0 {
            state.backlog = state.backlog.saturating_add(1);
            state.queue.push_back(processor);
            self.shared.wake.notify_one();
            return;
        }
        let close_inline = !state.closed;
        drop(state);
        if close_inline {
            shutdown_quietly(&processor);
            return;
        }
        // Past close the span-ending thread must not park on a flush shutdown no longer waits for.
        let straggler = thread::Builder::new()
            .name("otel-destination-drain-straggler".to_owned())
            .spawn(move || shutdown_quietly(&processor));
        if let Err(error) = straggler {
            tracing::error!(%error, "otel: could not start a destination drain straggler");
        }
    }

    #[must_use]
    pub fn saturated(&self) -> bool {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .backlog
            >= self.capacity
    }

    pub fn close(&self, timeout: Duration) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return;
        }
        state.closed = true;
        self.shared.wake.notify_all();
        let deadline = Instant::now()
            .checked_add(timeout)
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
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.closed = true;
        self.shared.wake.notify_all();
        drop(state);
    }
}

fn drain_until_closed(shared: &DrainShared) {
    while let Some(processor) = next_processor(shared) {
        shutdown_quietly(&processor);
        let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.backlog = state.backlog.saturating_sub(1);
        shared.wake.notify_all();
        drop(state);
    }
}

// A closed worker that finds the queue empty retires here and reports itself gone.
fn next_processor(shared: &DrainShared) -> Option<Arc<dyn SpanProcessor>> {
    let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
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

fn shutdown_quietly(processor: &Arc<dyn SpanProcessor>) {
    if let Err(error) = processor.shutdown() {
        tracing::debug!(%error, "otel: closing a destination processor failed");
    }
}

#[path = "drain/tests.rs"]
mod tests;
