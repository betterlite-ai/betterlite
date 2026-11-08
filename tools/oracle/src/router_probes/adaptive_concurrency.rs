pub mod wire;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_router::adaptive_concurrency::error::AcquireError;
use betterlite_router::adaptive_concurrency::{AdaptiveConcurrencyLimiter, Permit, PutSample};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio::time::Instant;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

use self::wire::{AcquireOutcome, PollOutcome, StateSnapshot, Step, StepResult};

const MAX_FILL: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub initial: u64,
    pub floor: u64,
    pub ceiling: u64,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub results: Vec<StepResult>,
}

enum Acquired<'limiter> {
    Pending(Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>>),
    Granted(Permit),
    Finished,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    let _entered = runtime.enter();
    let limiter = AdaptiveConcurrencyLimiter::new(input.initial, input.floor, input.ceiling)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let mut acquisitions: BTreeMap<usize, Acquired<'_>> = BTreeMap::new();
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(execute_step(step, &limiter, &mut acquisitions)?);
    }
    Ok(ProbeOutput::AdaptiveConcurrency(Output { results }))
}

fn execute_step<'limiter>(
    step: &Step,
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
) -> Result<StepResult, OperationError> {
    match step {
        Step::Snapshot => Ok(StepResult::Snapshot {
            state: state(limiter),
        }),
        Step::Record { throttled } => {
            limiter.record(PutSample {
                throttled: *throttled,
            });
            Ok(StepResult::Record {
                state: state(limiter),
            })
        }
        Step::Acquire { deadline_millis } => acquire(limiter, acquisitions, *deadline_millis),
        Step::Poll { id } => poll(limiter, acquisitions, *id),
        Step::Release { id } => release(limiter, acquisitions, *id),
        Step::Cancel { id } => cancel(limiter, acquisitions, *id),
        Step::Fill {
            count,
            deadline_millis,
        } => fill(limiter, acquisitions, *count, *deadline_millis),
    }
}

fn acquire<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    deadline_millis: u64,
) -> Result<StepResult, OperationError> {
    let (id, outcome) = start(limiter, acquisitions, deadline_millis)?;
    Ok(StepResult::Acquire {
        id,
        outcome,
        state: state(limiter),
    })
}

fn start<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    deadline_millis: u64,
) -> Result<(usize, AcquireOutcome), OperationError> {
    let id = acquisitions.len();
    let deadline = deadline(deadline_millis)?;
    let mut future: Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>> =
        Box::pin(limiter.acquire(deadline));
    let outcome = match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(Ok(permit)) => {
            drop(acquisitions.insert(id, Acquired::Granted(permit)));
            AcquireOutcome::Granted
        }
        Poll::Ready(Err(AcquireError::Overloaded)) => {
            drop(acquisitions.insert(id, Acquired::Finished));
            AcquireOutcome::Overloaded
        }
        Poll::Ready(Err(AcquireError::DeadlineExceeded)) => {
            drop(acquisitions.insert(id, Acquired::Finished));
            AcquireOutcome::TimedOut
        }
        Poll::Pending => {
            drop(acquisitions.insert(id, Acquired::Pending(future)));
            AcquireOutcome::Queued
        }
    };
    Ok((id, outcome))
}

fn poll<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    id: usize,
) -> Result<StepResult, OperationError> {
    let Some(entry) = acquisitions.remove(&id) else {
        return Err(not_started());
    };
    let mut future = match entry {
        Acquired::Pending(future) => future,
        other @ (Acquired::Granted(_) | Acquired::Finished) => {
            drop(acquisitions.insert(id, other));
            return Err(OperationError::InvalidRequest {
                message: "poll names an acquisition that is not waiting".to_owned(),
            });
        }
    };
    let (replacement, outcome) = match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(Ok(permit)) => (Acquired::Granted(permit), PollOutcome::Granted),
        Poll::Ready(Err(AcquireError::DeadlineExceeded)) => {
            (Acquired::Finished, PollOutcome::TimedOut)
        }
        Poll::Ready(Err(AcquireError::Overloaded)) => {
            drop(acquisitions.insert(id, Acquired::Finished));
            return Err(OperationError::InvalidRequest {
                message: "a queued acquisition cannot become overloaded".to_owned(),
            });
        }
        Poll::Pending => (Acquired::Pending(future), PollOutcome::Queued),
    };
    drop(acquisitions.insert(id, replacement));
    Ok(StepResult::Poll {
        id,
        outcome,
        state: state(limiter),
    })
}

fn release<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    id: usize,
) -> Result<StepResult, OperationError> {
    let Some(entry) = acquisitions.remove(&id) else {
        return Err(not_started());
    };
    let permit = match entry {
        Acquired::Granted(permit) => permit,
        other @ (Acquired::Pending(_) | Acquired::Finished) => {
            drop(acquisitions.insert(id, other));
            return Err(OperationError::InvalidRequest {
                message: "release names an acquisition that holds no permit".to_owned(),
            });
        }
    };
    drop(permit);
    drop(acquisitions.insert(id, Acquired::Finished));
    Ok(StepResult::Release {
        id,
        state: state(limiter),
    })
}

fn cancel<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    id: usize,
) -> Result<StepResult, OperationError> {
    let Some(entry) = acquisitions.remove(&id) else {
        return Err(not_started());
    };
    match entry {
        Acquired::Pending(future) => drop(future),
        other @ (Acquired::Granted(_) | Acquired::Finished) => {
            drop(acquisitions.insert(id, other));
            return Err(OperationError::InvalidRequest {
                message: "cancel names an acquisition that is not waiting".to_owned(),
            });
        }
    }
    drop(acquisitions.insert(id, Acquired::Finished));
    Ok(StepResult::Cancel {
        id,
        state: state(limiter),
    })
}

fn fill<'limiter>(
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: &mut BTreeMap<usize, Acquired<'limiter>>,
    count: u64,
    deadline_millis: u64,
) -> Result<StepResult, OperationError> {
    if count > MAX_FILL {
        return Err(OperationError::InvalidRequest {
            message: "a fill is limited to 4096 acquisitions".to_owned(),
        });
    }
    let mut queued = 0_u64;
    let mut overloaded = 0_u64;
    let mut granted = 0_u64;
    let mut timed_out = 0_u64;
    for _ in 0..count {
        match start(limiter, acquisitions, deadline_millis)?.1 {
            AcquireOutcome::Queued => queued = queued.saturating_add(1),
            AcquireOutcome::Overloaded => overloaded = overloaded.saturating_add(1),
            AcquireOutcome::Granted => granted = granted.saturating_add(1),
            AcquireOutcome::TimedOut => timed_out = timed_out.saturating_add(1),
        }
    }
    Ok(StepResult::Fill {
        queued,
        overloaded,
        granted,
        timed_out,
        state: state(limiter),
    })
}

fn state(limiter: &AdaptiveConcurrencyLimiter) -> StateSnapshot {
    let snapshot = limiter.snapshot();
    StateSnapshot {
        limit: snapshot.limit,
        in_flight: snapshot.in_flight,
        waiters: snapshot.waiters,
        clean_streak: snapshot.clean_streak,
    }
}

fn deadline(millis: u64) -> Result<Instant, OperationError> {
    Instant::now()
        .checked_add(Duration::from_millis(millis))
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "the deadline is too far in the future".to_owned(),
        })
}

fn not_started() -> OperationError {
    OperationError::InvalidRequest {
        message: "the step names an acquisition that was never started".to_owned(),
    }
}
