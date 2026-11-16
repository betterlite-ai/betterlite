use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use super::LifecycleFuture;
use crate::auth_reconnect::clock::{ReconnectClock, SystemClock};

pub const READ_ONLY_RECREATE_BACKOFF_CAP: Duration = Duration::from_secs(600);
const MAX_BACKOFF_EXPONENT: u32 = 10;

pub trait ReconnectControl: Send + Sync {
    fn reconnect<'borrow>(
        &'borrow self,
        reason: &'borrow str,
        timeout: Option<Duration>,
    ) -> LifecycleFuture<'borrow, bool>;

    fn force_recreate<'borrow>(
        &'borrow self,
        reason: &'borrow str,
        timeout: Option<Duration>,
    ) -> LifecycleFuture<'borrow, bool>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecreateState {
    pub last_recreate: Option<Instant>,
    pub streak: u32,
}

pub struct ReadOnlyWriter<Reconnect, Clock = SystemClock> {
    pub reconnect: Arc<Reconnect>,
    pub cooldown: Duration,
    pub clock: Clock,
    pub state: Mutex<RecreateState>,
}

enum RecreateDecision {
    BackOff { streak: u32, wait: Duration },
    Recreate,
}

impl<Reconnect, Clock> ReadOnlyWriter<Reconnect, Clock>
where
    Reconnect: ReconnectControl,
    Clock: ReconnectClock,
{
    #[must_use]
    pub async fn recreate(&self, reason: &str, timeout: Option<Duration>) -> bool {
        match self.decide() {
            RecreateDecision::BackOff { streak, wait } => {
                debug!(
                    "Writer session still read-only after {streak} recreate(s); backing off {}s. reason={reason}",
                    wait.as_secs_f64()
                );
                false
            }
            RecreateDecision::Recreate => {
                warn!(
                    "Writer session is read-only (writes fail with SQLSTATE 25006); recreating Prisma client. reason={reason}"
                );
                self.reconnect.force_recreate(reason, timeout).await
            }
        }
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.last_recreate = None;
        state.streak = 0;
    }

    fn decide(&self) -> RecreateDecision {
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let wait = backoff_wait(self.cooldown, state.streak);
        if state
            .last_recreate
            .is_some_and(|last| now.saturating_duration_since(last) < wait)
        {
            return RecreateDecision::BackOff {
                streak: state.streak,
                wait,
            };
        }
        state.last_recreate = Some(now);
        state.streak = state.streak.saturating_add(1);
        RecreateDecision::Recreate
    }
}

fn backoff_wait(cooldown: Duration, streak: u32) -> Duration {
    let factor = 2_u32
        .checked_pow(streak.min(MAX_BACKOFF_EXPONENT))
        .unwrap_or(u32::MAX);
    cooldown
        .saturating_mul(factor)
        .min(READ_ONLY_RECREATE_BACKOFF_CAP)
}
