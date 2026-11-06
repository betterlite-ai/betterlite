use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
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

// the lock is released around the call, so a concurrent caller and the call itself may use the policy
struct CallScope<'policy, C: ?Sized> {
    policy: &'policy Mutex<RedisFailurePolicy>,
    clock: &'policy C,
    admission: Admission,
}

impl<'policy, C> CallScope<'policy, C>
where
    C: Fn() -> Duration + ?Sized,
{
    fn enter(
        policy: &'policy Mutex<RedisFailurePolicy>,
        clock: &'policy C,
        operation: &str,
    ) -> Result<Self, CircuitOpenError> {
        let admission = {
            let mut breaker = policy.lock().unwrap_or_else(PoisonError::into_inner);
            breaker.admit(clock(), operation)?
        };
        Ok(Self {
            policy,
            clock,
            admission,
        })
    }

    fn settle<T, E>(&self, result: Result<T, E>) -> Result<CallOutcome<T>, CallError<E>>
    where
        E: RedisCallError,
    {
        match result {
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
        self.policy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(self.admission)
    }

    fn record<E>(&self, error: &E) -> Option<BreakerEvent>
    where
        E: RedisCallError,
    {
        let failure = error.failure()?;
        let mut breaker = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        breaker.record_failure((self.clock)(), failure)
    }
}

pub fn run<T, E, F>(
    policy: &Mutex<RedisFailurePolicy>,
    clock: &(dyn Fn() -> Duration + Sync),
    operation: &str,
    call: F,
) -> Result<CallOutcome<T>, CallError<E>>
where
    F: FnOnce() -> Result<T, E> + Send,
    T: Send,
    E: RedisCallError + Send,
{
    let mut future = Box::pin(run_async(policy, clock, operation, || ready(call())));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    // the sync call is ready on its first poll, so this loop ends after one pass
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

pub async fn run_async<T, E, F, Fut, C>(
    policy: &Mutex<RedisFailurePolicy>,
    clock: &C,
    operation: &str,
    call: F,
) -> Result<CallOutcome<T>, CallError<E>>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<T, E>> + Send,
    E: RedisCallError,
    C: Fn() -> Duration + Sync + ?Sized,
{
    let scope = match CallScope::enter(policy, clock, operation) {
        Ok(scope) => scope,
        Err(error) => return Err(CallError::Open(error)),
    };
    let result = call().await;
    scope.settle(result)
}
