use std::time::Duration;

use tokio::time::Instant;

// PostgreSQL refuses a statement_timeout or lock_timeout above INT_MAX milliseconds
pub const MAX_TIMEOUT_MS: u32 = i32::MAX.unsigned_abs();

// the run budget is wall-clock time measured on this clock, and tests pass one they move by hand
pub trait MonotonicClock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokioClock;

impl MonotonicClock for TokioClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

// _timeout_ms: SET takes no bind parameter, so the bound is an integer of milliseconds and at least 1
#[must_use]
pub fn statement_timeout_ms(now: Instant, deadline: Instant, batch_timeout: Duration) -> u64 {
    let remaining = deadline.saturating_duration_since(now);
    let bound = batch_timeout.min(remaining).as_millis();
    let ceiling = u64::from(MAX_TIMEOUT_MS);
    u64::try_from(bound).unwrap_or(ceiling).clamp(1, ceiling)
}

// _group_deadline: each pending group gets an equal share of what is left, so one backlog cannot starve the next
#[must_use]
pub fn group_deadline(overall: Instant, now: Instant, groups_remaining: u32) -> Instant {
    if now >= overall {
        return overall;
    }
    overall
        .saturating_duration_since(now)
        .checked_div(groups_remaining)
        .and_then(|share| now.checked_add(share))
        .unwrap_or(overall)
}

// _remaining_timeout_ms: a fresh bound per delegated statement, and None once the deadline has passed
pub fn remaining_budget<Clock>(
    clock: &Clock,
    deadline: Instant,
    batch_timeout: Duration,
) -> impl FnMut() -> Option<u64> + Send + '_
where
    Clock: MonotonicClock,
{
    move || {
        let now = clock.now();
        (now < deadline).then(|| statement_timeout_ms(now, deadline, batch_timeout))
    }
}
