use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use tracing::{debug, error, info, warn};

use super::clock::{ReconnectClock, SystemClock};
use super::engine::ReconnectEngine;
use super::lock::{ReconnectLock, TokioLock};
use super::request::{ReconnectError, ReconnectHistory, ReconnectRequest, ReconnectSnapshot};
use super::settings::ReconnectSettings;
use super::stale::StaleReadEngine;

mod cycle;

struct FailedRepair<Handle> {
    handle: Arc<Handle>,
    generation: u64,
}

pub struct AuthReconnect<Engine, Clock = SystemClock, Gate = TokioLock>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    engine: Engine,
    settings: ReconnectSettings,
    clock: Clock,
    lock: Gate,
    last_attempt: Mutex<Option<Instant>>,
    failures: AtomicU32,
    failed_repairs: Mutex<Vec<FailedRepair<Engine::Handle>>>,
    dead: AtomicBool,
}

impl<Engine, Clock, Gate> AuthReconnect<Engine, Clock, Gate>
where
    Engine: ReconnectEngine,
    Clock: ReconnectClock,
    Gate: ReconnectLock,
{
    #[must_use]
    pub fn new(
        engine: Engine,
        settings: ReconnectSettings,
        clock: Clock,
        lock: Gate,
        history: ReconnectHistory<Engine::Handle>,
    ) -> Self {
        let failed_repairs = history
            .failed_repairs
            .into_iter()
            .map(|(handle, generation)| FailedRepair { handle, generation })
            .collect();
        Self {
            engine,
            settings,
            clock,
            lock,
            last_attempt: Mutex::new(history.last_attempt),
            failures: AtomicU32::new(history.consecutive_failures),
            failed_repairs: Mutex::new(failed_repairs),
            dead: AtomicBool::new(history.engine_dead),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> ReconnectSnapshot {
        ReconnectSnapshot {
            consecutive_failures: self.failures.load(Ordering::SeqCst),
            engine_dead: self.dead.load(Ordering::SeqCst),
        }
    }

    #[must_use]
    pub fn has_failed_repair_for(&self, handle: &Arc<Engine::Handle>, generation: u64) -> bool {
        let failed_repairs = self
            .failed_repairs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        failed_repairs
            .iter()
            .any(|repair| Arc::ptr_eq(&repair.handle, handle) && repair.generation == generation)
    }

    #[must_use]
    pub async fn attempt_db_reconnect(
        &self,
        request: &ReconnectRequest<'_>,
        stale: Option<&StaleReadEngine<Engine::Handle>>,
    ) -> bool {
        let now = self.clock.now();
        if !request.force && self.cooldown_blocks(stale, now) {
            debug!(
                reason = request.reason,
                "Skipping DB reconnect attempt due to cooldown."
            );
            return false;
        }
        let hold = match self.lock.acquire(request.lock_timeout).await {
            Ok(Some(hold)) => hold,
            Ok(None) => {
                debug!(
                    reason = request.reason,
                    timeout = ?request.lock_timeout.unwrap_or_default().as_secs_f64(),
                    "Skipping DB reconnect attempt due to lock acquisition timeout."
                );
                return false;
            }
            Err(error) => {
                debug!(
                    reason = request.reason,
                    error = %error,
                    "Skipping DB reconnect attempt due to lock acquisition error."
                );
                return false;
            }
        };
        let completed = self.attempt_inside_lock(request, stale).await;
        drop(hold);
        completed
    }

    fn cooldown_blocks(
        &self,
        stale: Option<&StaleReadEngine<Engine::Handle>>,
        now: Instant,
    ) -> bool {
        let within = {
            let last_attempt = self
                .last_attempt
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            last_attempt
                .is_some_and(|last| now.saturating_duration_since(last) < self.settings.cooldown)
        };
        if !within {
            return false;
        }
        stale.is_none_or(|stale| {
            self.has_failed_repair_for(&stale.handle, stale.generation)
                || !stale.is_still_live(&self.engine.read_target())
        })
    }

    fn record_failed_repair(&self, stale: &StaleReadEngine<Engine::Handle>) {
        let mut failed_repairs = self
            .failed_repairs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(repair) = failed_repairs
            .iter_mut()
            .find(|repair| Arc::ptr_eq(&repair.handle, &stale.handle))
        {
            repair.generation = stale.generation;
            return;
        }
        if failed_repairs.len() >= 2 {
            failed_repairs.remove(0);
        }
        failed_repairs.push(FailedRepair {
            handle: Arc::clone(&stale.handle),
            generation: stale.generation,
        });
    }

    async fn attempt_inside_lock(
        &self,
        request: &ReconnectRequest<'_>,
        stale: Option<&StaleReadEngine<Engine::Handle>>,
    ) -> bool {
        let now = self.clock.now();
        if !request.force && self.cooldown_blocks(stale, now) {
            debug!(
                reason = request.reason,
                "Skipping DB reconnect attempt inside lock due to cooldown."
            );
            return false;
        }
        let failures = self.failures.load(Ordering::SeqCst);
        if u64::from(failures) >= self.settings.escalation_threshold {
            warn!(
                consecutive = %failures,
                reason = request.reason,
                "Escalating to heavy reconnect after consecutive failures."
            );
            self.dead.store(true, Ordering::SeqCst);
        }
        warn!(reason = request.reason, "Attempting Prisma DB reconnect.");
        let outcome = self.run_cycle(request).await;
        *self
            .last_attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(self.clock.now());
        self.settle(request, stale, outcome)
    }

    fn settle(
        &self,
        request: &ReconnectRequest<'_>,
        stale: Option<&StaleReadEngine<Engine::Handle>>,
        outcome: Result<(), ReconnectError<Engine::Error>>,
    ) -> bool {
        match outcome {
            Ok(()) => {
                self.failures.store(0, Ordering::SeqCst);
                info!(reason = request.reason, "Prisma DB reconnect succeeded.");
                true
            }
            Err(ReconnectError::ForcedDecline) => {
                self.failures.store(0, Ordering::SeqCst);
                warn!(reason = request.reason, "Prisma DB reconnect declined.");
                false
            }
            Err(error) => {
                let consecutive = self
                    .failures
                    .fetch_add(1, Ordering::SeqCst)
                    .saturating_add(1);
                if let Some(stale) = stale {
                    self.record_failed_repair(stale);
                }
                error!(
                    consecutive = %consecutive,
                    reason = request.reason,
                    error = %error,
                    "Prisma DB reconnect failed."
                );
                false
            }
        }
    }
}
