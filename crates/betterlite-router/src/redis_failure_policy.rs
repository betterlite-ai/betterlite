pub mod classify;
pub mod error;
pub mod log;
pub mod run;
mod tests;

use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use serde::{Deserialize, Serialize};

use self::classify::{Failure, FailureClass, FailureKind, failure_kind, is_timeout_failure};
use self::error::CircuitOpenError;
use self::log::{LogDecision, TimeoutLogThrottle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub failure_threshold: u32,
    pub recovery_timeout: Duration,
    pub timeout_min_duration: Duration,
    pub timeout_log_interval: Duration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

impl State {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: State,
    pub generation: u64,
    pub failure_count: u32,
    pub hard_failure_count: u32,
    pub timeout_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    generation: u64,
    swallowed: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BreakerEvent {
    Opened {
        failure_count: u32,
        hard_failure_count: u32,
        recovery_timeout: Duration,
    },
    Recovered,
}

pub struct RedisFailurePolicy {
    settings: Settings,
    metrics: BreakerMetrics,
    state: State,
    generation: u64,
    failure_count: u32,
    hard_failure_count: u32,
    timeout_count: u32,
    timeout_streak_started: Option<Duration>,
    opened_at: Duration,
    probe_in_flight: bool,
    timeout_log: TimeoutLogThrottle,
}

impl RedisFailurePolicy {
    #[must_use]
    pub fn new(settings: Settings, metrics: BreakerMetrics) -> Self {
        metrics.record_state_change(None, State::Closed.label());
        Self {
            settings,
            metrics,
            state: State::Closed,
            generation: 0,
            failure_count: 0,
            hard_failure_count: 0,
            timeout_count: 0,
            timeout_streak_started: None,
            opened_at: Duration::ZERO,
            probe_in_flight: false,
            timeout_log: TimeoutLogThrottle {
                interval: settings.timeout_log_interval,
                last_logged_at: None,
                suppressed: 0,
            },
        }
    }

    #[must_use]
    pub const fn status(&self) -> Status {
        Status {
            state: self.state,
            generation: self.generation,
            failure_count: self.failure_count,
            hard_failure_count: self.hard_failure_count,
            timeout_count: self.timeout_count,
        }
    }

    pub fn admit(&mut self, now: Duration, operation: &str) -> Result<Admission, CircuitOpenError> {
        if !self.settings.enabled {
            return Ok(Admission {
                generation: self.generation,
                swallowed: 0,
            });
        }
        let admitted = match self.state {
            State::Closed => true,
            State::HalfOpen => {
                if self.probe_in_flight {
                    false
                } else {
                    self.probe_in_flight = true;
                    true
                }
            }
            State::Open => {
                let elapsed = now.saturating_sub(self.opened_at);
                if elapsed > self.settings.recovery_timeout {
                    self.set_state(State::HalfOpen);
                    self.probe_in_flight = true;
                    true
                } else {
                    false
                }
            }
        };
        if !admitted {
            return Err(CircuitOpenError {
                operation: operation.to_owned(),
            });
        }
        Ok(Admission {
            generation: self.generation,
            swallowed: 0,
        })
    }

    pub fn record_failure(&mut self, now: Duration, failure: Failure<'_>) -> Option<BreakerEvent> {
        if !self.settings.enabled {
            return None;
        }
        let kind = failure_kind(failure.class, failure.causes)?;
        self.count_failure(now, kind)
    }

    pub fn record_swallowed_failure(
        &mut self,
        admission: &mut Admission,
        now: Duration,
        failure: Failure<'_>,
    ) -> Option<BreakerEvent> {
        let kind = failure_kind(failure.class, failure.causes)?;
        admission.swallowed = admission.swallowed.saturating_add(1);
        if !self.settings.enabled {
            return None;
        }
        self.count_failure(now, kind)
    }

    pub fn finish(&mut self, admission: Admission) -> Option<BreakerEvent> {
        if admission.swallowed > 0 {
            return None;
        }
        if admission.generation != self.generation {
            return None;
        }
        self.record_success()
    }

    pub fn release(&mut self, admission: Admission) {
        if self.state == State::HalfOpen && admission.generation == self.generation {
            self.probe_in_flight = false;
        }
    }

    #[must_use]
    pub fn log_failure(&mut self, now: Duration, failure: Failure<'_>) -> LogDecision {
        if failure.class == FailureClass::CircuitOpen {
            return LogDecision::Debug;
        }
        if !is_timeout_failure(failure.class, failure.causes) {
            return LogDecision::Level { suppressed: 0 };
        }
        self.timeout_log
            .admit(now)
            .map_or(LogDecision::Debug, |suppressed| LogDecision::Level {
                suppressed,
            })
    }

    fn count_failure(&mut self, now: Duration, kind: FailureKind) -> Option<BreakerEvent> {
        self.failure_count = self.failure_count.saturating_add(1);
        match kind {
            FailureKind::Timeout => {
                self.timeout_count = self.timeout_count.saturating_add(1);
                if self.timeout_streak_started.is_none() {
                    self.timeout_streak_started = Some(now);
                }
            }
            FailureKind::Connectivity => {
                self.hard_failure_count = self.hard_failure_count.saturating_add(1);
                self.timeout_count = 0;
                self.timeout_streak_started = None;
            }
        }
        self.opened_at = now;
        self.metrics.record_failure(kind.label());
        if !self.should_open(now) {
            return None;
        }
        let opened = self.state != State::Open;
        self.set_state(State::Open);
        opened.then_some(BreakerEvent::Opened {
            failure_count: self.failure_count,
            hard_failure_count: self.hard_failure_count,
            recovery_timeout: self.settings.recovery_timeout,
        })
    }

    pub fn record_success(&mut self) -> Option<BreakerEvent> {
        if !self.settings.enabled || self.state == State::Open {
            return None;
        }
        let recovered = self.state == State::HalfOpen;
        self.failure_count = 0;
        self.hard_failure_count = 0;
        self.timeout_count = 0;
        self.timeout_streak_started = None;
        self.set_state(State::Closed);
        recovered.then_some(BreakerEvent::Recovered)
    }

    fn should_open(&self, now: Duration) -> bool {
        if self.state == State::HalfOpen {
            return true;
        }
        if self.hard_failure_count >= self.settings.failure_threshold {
            return true;
        }
        if self.timeout_count < self.settings.failure_threshold {
            return false;
        }
        let started = self.timeout_streak_started.unwrap_or(now);
        now.saturating_sub(started) >= self.settings.timeout_min_duration
    }

    fn set_state(&mut self, state: State) {
        if state == self.state {
            return;
        }
        self.metrics.record_transition(state.label());
        self.metrics
            .record_state_change(Some(self.state.label()), state.label());
        self.state = state;
        self.generation = self.generation.saturating_add(1);
    }
}
