use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::{Instant, sleep};

use super::fetch::{RunCredentials, RunTransport, fetch_run};
use super::records::TinyfishRun;
use super::{
    TINYFISH_MAX_POLL_FAILURES, TINYFISH_MAX_POLL_SECONDS, TINYFISH_POLL_SECONDS,
    TINYFISH_TERMINAL_STATUSES,
};

pub type PollSleepFuture<'call> = Pin<Box<dyn Future<Output = ()> + Send + 'call>>;

// LiteLLM reads the deadline off time.monotonic, so the origin is fixed once per poller and never reset.
pub trait PollClock: Send + Sync {
    fn now(&self) -> Duration;
}

pub trait PollSleeper: Send + Sync {
    fn sleep(&self, duration: Duration) -> PollSleepFuture<'_>;
}

#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    pub origin: Instant,
}

impl PollClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokioSleeper;

impl PollSleeper for TokioSleeper {
    fn sleep(&self, duration: Duration) -> PollSleepFuture<'_> {
        Box::pin(sleep(duration))
    }
}

pub struct Poller<'poll> {
    pub credentials: &'poll dyn RunCredentials,
    pub transport: &'poll dyn RunTransport,
    pub clock: &'poll dyn PollClock,
    pub sleeper: &'poll dyn PollSleeper,
    pub api_base: &'poll str,
}

impl Poller<'_> {
    // The deadline is absolute, so failed polls never extend it, and the last run seen is what gets billed.
    pub async fn poll_until_terminal(&self, run_id: &str) -> Option<TinyfishRun> {
        let interval = TINYFISH_POLL_SECONDS;
        let budget = TINYFISH_MAX_POLL_SECONDS;
        let deadline = self.clock.now().saturating_add(budget);
        let mut last_run = None;
        let mut consecutive_failures = 0;
        while self.clock.now() < deadline {
            let seen = fetch_run(self.credentials, self.transport, self.api_base, run_id).await;
            match poll_step(run_id, seen, &mut consecutive_failures) {
                PollStep::Terminal(run) => return Some(run),
                PollStep::GaveUp => return last_run,
                PollStep::Continue(Some(run)) => last_run = Some(run),
                PollStep::Continue(None) => {}
            }
            self.sleeper.sleep(interval).await;
        }
        tracing::warn!(
            "TinyFish passthrough: run {run_id} not terminal after {}s; logging the request without cost",
            budget.as_secs()
        );
        last_run
    }
}

// A fetched run resets the failure count, and a run without a terminal status is only remembered.
fn poll_step(run_id: &str, seen: Option<TinyfishRun>, failures: &mut u32) -> PollStep {
    let Some(run) = seen else {
        *failures = failures.saturating_add(1);
        if *failures >= TINYFISH_MAX_POLL_FAILURES {
            tracing::warn!(
                "TinyFish passthrough: giving up on run {run_id} after {} consecutive poll failures; logging the request without cost",
                *failures
            );
            return PollStep::GaveUp;
        }
        return PollStep::Continue(None);
    };
    *failures = 0;
    if is_terminal(&run) {
        return PollStep::Terminal(run);
    }
    PollStep::Continue(Some(run))
}

enum PollStep {
    Terminal(TinyfishRun),
    Continue(Option<TinyfishRun>),
    GaveUp,
}

// LiteLLM reads the status as text and compares it to the three terminal values, so a null or absent status keeps polling.
pub(super) fn is_terminal(run: &TinyfishRun) -> bool {
    let status = run
        .status
        .as_ref()
        .and_then(|value| value.as_deref())
        .unwrap_or_default();
    TINYFISH_TERMINAL_STATUSES.contains(&status)
}
