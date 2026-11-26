pub(crate) mod failure_class;

mod tests;

use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;

use self::failure_class::classify;
use super::classify::{Failure, FailureClass, is_health_failure};
use super::log::{FailureLogLevel, log_breaker_event, log_redis_failure};
use super::{Admission, RedisFailurePolicy, State};
use crate::routing_state_store::BreakerClock;

// An admission is the generation times 2^32 plus the call number, and neither reaches 2^32 in a process.
const CALL_SPACE: u64 = 0x1_0000_0000;

// A call that swallows a failure and never exits leaves its number here until an exit or the cap drops it.
pub(crate) const SWALLOWED_CALLS_KEPT: usize = 4096;

// One breaker per cache, as LiteLLM gives every RedisCache instance its own RedisCircuitBreaker.
pub struct PolicyGate {
    pub(crate) policy: Arc<Mutex<RedisFailurePolicy>>,
    pub(crate) next_call: AtomicU64,
    pub(crate) swallowed_calls: Mutex<BTreeSet<u64>>,
    pub(crate) probe: Mutex<Option<ProbeSlot>>,
    pub(crate) clock: BreakerClock,
}

// Releasing the half open probe on drop keeps a call that never exits from holding the only probe slot.
pub(crate) struct ProbeSlot {
    policy: Arc<Mutex<RedisFailurePolicy>>,
    admission: Admission,
}

impl Drop for ProbeSlot {
    fn drop(&mut self) {
        self.policy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .release(self.admission);
    }
}

impl BreakerGate for PolicyGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        // a probe whose future was dropped never reaches exit, so the next call frees its slot instead of waiting
        self.release_probe();
        let now = (self.clock)();
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        let admission = policy
            .admit(now, name)
            .map_err(|_open| BreakerOpen { name })?;
        let is_probe = policy.status().state == State::HalfOpen;
        let generation = policy.status().generation;
        drop(policy);
        if is_probe {
            let slot = ProbeSlot {
                policy: Arc::clone(&self.policy),
                admission,
            };
            let replaced = self
                .probe
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .replace(slot);
            drop(replaced);
        }
        let call = self.next_call.fetch_add(1, Ordering::Relaxed);
        Ok(BreakerAdmission::from(
            generation
                .wrapping_mul(CALL_SPACE)
                .wrapping_add(call.rem_euclid(CALL_SPACE)),
        ))
    }

    fn exit(&self, admission: BreakerAdmission) {
        let bits = *admission.as_ref();
        let generation = bits.div_euclid(CALL_SPACE);
        let call = bits.rem_euclid(CALL_SPACE);
        let swallowed = self
            .swallowed_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&call);
        if swallowed {
            return;
        }
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        if policy.status().generation != generation {
            return;
        }
        if let Some(event) = policy.record_success() {
            log_breaker_event(&event);
        }
    }

    fn record_failure(&self, error: &(dyn StdError + Send + Sync)) {
        let Some((class, causes)) = Self::health_failure(error) else {
            // a call ending without a health signal must not keep a half open probe slot taken
            self.release_probe();
            return;
        };
        self.record(
            Failure {
                class,
                causes: &causes,
            },
            error,
        );
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn StdError + Send + Sync)) {
        let Some((class, causes)) = Self::health_failure(error) else {
            // the call may still exit, but its non-health error must not keep a half open probe slot taken
            self.release_probe();
            return;
        };
        self.record(
            Failure {
                class,
                causes: &causes,
            },
            error,
        );
        let call = admission.as_ref().rem_euclid(CALL_SPACE);
        let mut kept = self
            .swallowed_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        kept.insert(call);
        let excess = kept.len().saturating_sub(SWALLOWED_CALLS_KEPT);
        for _ in 0..excess {
            let _dropped = kept.pop_first();
        }
    }
}

impl PolicyGate {
    #[must_use]
    pub fn new(policy: RedisFailurePolicy, clock: BreakerClock) -> Self {
        Self {
            policy: Arc::new(Mutex::new(policy)),
            next_call: AtomicU64::new(0),
            swallowed_calls: Mutex::new(BTreeSet::new()),
            probe: Mutex::new(None),
            clock,
        }
    }

    fn release_probe(&self) {
        // the guard drops after the probe lock is released, so its release takes only the policy lock
        let stale = self
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(stale);
    }

    fn health_failure(
        error: &(dyn StdError + Send + Sync),
    ) -> Option<(FailureClass, Vec<FailureClass>)> {
        let (class, causes) = classify(error)?;
        is_health_failure(class).then_some((class, causes))
    }

    // The outermost class decides, and a deeper cause only marks a connection failure as a timeout.
    fn record(&self, failure: Failure<'_>, error: &(dyn StdError + Send + Sync)) {
        let now = (self.clock)();
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        let event = policy.record_failure(now, failure);
        let decision = policy.log_failure(now, failure);
        drop(policy);
        if let Some(opened) = event {
            log_breaker_event(&opened);
        }
        log_redis_failure(
            decision,
            FailureLogLevel::Error,
            "Redis operation failed",
            &error.to_string(),
        );
    }
}
