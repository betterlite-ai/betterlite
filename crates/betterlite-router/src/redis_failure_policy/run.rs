use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::classify::Failure;
use super::error::CircuitOpenError;
use super::{Admission, BreakerEvent, RedisFailurePolicy};

mod tests;

pub trait RedisCallError {
    fn failure(&self) -> Option<Failure<'_>>;
}

#[derive(Debug)]
pub enum CallError<E> {
    Open(CircuitOpenError),
    Failed {
        error: E,
        warning: Option<BreakerEvent>,
    },
}

#[derive(Debug)]
pub struct CallOutcome<T> {
    pub value: T,
    pub warning: Option<BreakerEvent>,
}

// the policy lock is released around the call, so a concurrent caller and the call itself may use it
pub struct CallGuard<'policy, C: ?Sized> {
    policy: &'policy Mutex<RedisFailurePolicy>,
    clock: &'policy C,
    admission: Arc<Mutex<Admission>>,
}

impl<C: ?Sized> Clone for CallGuard<'_, C> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            clock: self.clock,
            admission: Arc::clone(&self.admission),
        }
    }
}

impl<'policy, C> CallGuard<'policy, C>
where
    C: Fn() -> Duration + ?Sized,
{
    pub fn enter(
        policy: &'policy Mutex<RedisFailurePolicy>,
        clock: &'policy C,
        operation: &str,
    ) -> Result<Self, CircuitOpenError> {
        let admission = policy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(clock(), operation)?;
        Ok(Self {
            policy,
            clock,
            admission: Arc::new(Mutex::new(admission)),
        })
    }

    pub fn swallow(&self, failure: Failure<'_>) -> Option<BreakerEvent> {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        policy.record_swallowed_failure(&mut admission, (self.clock)(), failure)
    }

    pub fn call<T, E, F>(&self, call: F) -> Result<CallOutcome<T>, CallError<E>>
    where
        F: FnOnce(&Self) -> Result<T, E>,
        E: RedisCallError,
    {
        match call(self) {
            Ok(value) => Ok(CallOutcome {
                value,
                warning: self.finish(),
            }),
            Err(error) => {
                let warning = self.record(&error);
                Err(CallError::Failed { error, warning })
            }
        }
    }

    fn finish(&self) -> Option<BreakerEvent> {
        let admission = *self
            .admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.policy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(admission)
    }

    fn record<E>(&self, error: &E) -> Option<BreakerEvent>
    where
        E: RedisCallError,
    {
        let failure = error.failure()?;
        self.policy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record_failure((self.clock)(), failure)
    }
}

pub async fn run_async<'policy, T, E, F, Fut, C>(
    policy: &'policy Mutex<RedisFailurePolicy>,
    clock: &'policy C,
    operation: &str,
    invoke: F,
) -> Result<CallOutcome<T>, CallError<E>>
where
    F: FnOnce(CallGuard<'policy, C>) -> Fut + Send,
    Fut: Future<Output = Result<T, E>> + Send,
    E: RedisCallError,
    C: Fn() -> Duration + Sync + ?Sized,
{
    let guard = CallGuard::enter(policy, clock, operation).map_err(CallError::Open)?;
    let result = invoke(guard.clone()).await;
    guard.call(|_| result)
}
