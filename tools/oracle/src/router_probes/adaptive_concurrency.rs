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

struct Session<'limiter> {
    limiter: &'limiter AdaptiveConcurrencyLimiter,
    acquisitions: BTreeMap<usize, Acquired<'limiter>>,
    next_id: usize,
}

enum Acquired<'limiter> {
    Pending(Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>>),
    Granted(Permit),
}

#[derive(Clone, Copy)]
enum Withdraw {
    Release { id: usize },
    Cancel { id: usize },
}

impl Withdraw {
    const fn id(self) -> usize {
        match self {
            Self::Release { id } | Self::Cancel { id } => id,
        }
    }

    const fn rejection(self) -> &'static str {
        match self {
            Self::Release { .. } => "release names an acquisition that holds no permit",
            Self::Cancel { .. } => "cancel names an acquisition that is not waiting",
        }
    }
}

impl<'limiter> Session<'limiter> {
    fn snapshot(&self) -> StateSnapshot {
        let snapshot = self.limiter.snapshot();
        StateSnapshot {
            limit: snapshot.limit,
            in_flight: snapshot.in_flight,
            waiters: snapshot.waiters,
            clean_streak: snapshot.clean_streak,
        }
    }

    fn take(&mut self, id: usize) -> Result<Acquired<'limiter>, OperationError> {
        let Some(entry) = self.acquisitions.remove(&id) else {
            return Err(OperationError::InvalidRequest {
                message: "the step names an acquisition that was never started".to_owned(),
            });
        };
        Ok(entry)
    }

    fn put(&mut self, id: usize, entry: Acquired<'limiter>) {
        if let Some(replaced) = self.acquisitions.insert(id, entry) {
            drop(replaced);
        }
    }

    fn reject(&mut self, id: usize, entry: Acquired<'limiter>, message: &str) -> OperationError {
        self.put(id, entry);
        OperationError::InvalidRequest {
            message: message.to_owned(),
        }
    }
}

#[derive(Default)]
struct FillCounts {
    queued: u64,
    overloaded: u64,
    granted: u64,
    timed_out: u64,
}

impl FillCounts {
    const fn add(&mut self, outcome: AcquireOutcome) {
        match outcome {
            AcquireOutcome::Queued => self.queued = self.queued.saturating_add(1),
            AcquireOutcome::Overloaded => self.overloaded = self.overloaded.saturating_add(1),
            AcquireOutcome::Granted => self.granted = self.granted.saturating_add(1),
            AcquireOutcome::TimedOut => self.timed_out = self.timed_out.saturating_add(1),
        }
    }
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
    let mut session = Session {
        limiter: &limiter,
        acquisitions: BTreeMap::new(),
        next_id: 0,
    };
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(execute_step(step, &mut session)?);
    }
    Ok(ProbeOutput::AdaptiveConcurrency(Output { results }))
}

fn execute_step(step: &Step, session: &mut Session<'_>) -> Result<StepResult, OperationError> {
    match step {
        Step::Snapshot => Ok(StepResult::Snapshot {
            state: session.snapshot(),
        }),
        Step::Record { throttled } => Ok(record(session, *throttled)),
        Step::Acquire { deadline_millis } => acquire(session, *deadline_millis),
        Step::Poll { id } => poll(session, *id),
        Step::Release { id } => withdraw(session, Withdraw::Release { id: *id }),
        Step::Cancel { id } => withdraw(session, Withdraw::Cancel { id: *id }),
        Step::Fill {
            count,
            deadline_millis,
        } => fill(session, *count, *deadline_millis),
    }
}

fn record(session: &Session<'_>, throttled: bool) -> StepResult {
    session.limiter.record(PutSample { throttled });
    StepResult::Record {
        state: session.snapshot(),
    }
}

fn acquire(session: &mut Session<'_>, deadline_millis: u64) -> Result<StepResult, OperationError> {
    let (id, outcome) = start(session, deadline_millis)?;
    let state = session.snapshot();
    Ok(StepResult::Acquire { id, outcome, state })
}

fn start<'limiter>(
    session: &mut Session<'limiter>,
    deadline_millis: u64,
) -> Result<(usize, AcquireOutcome), OperationError> {
    let id = session.next_id;
    session.next_id = session.next_id.saturating_add(1);
    let deadline = deadline(deadline_millis)?;
    let mut future: Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>> =
        Box::pin(session.limiter.acquire(deadline));
    let outcome = match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(Ok(permit)) => {
            session.put(id, Acquired::Granted(permit));
            AcquireOutcome::Granted
        }
        Poll::Ready(Err(AcquireError::Overloaded)) => AcquireOutcome::Overloaded,
        Poll::Ready(Err(AcquireError::DeadlineExceeded)) => AcquireOutcome::TimedOut,
        Poll::Pending => {
            session.put(id, Acquired::Pending(future));
            AcquireOutcome::Queued
        }
    };
    Ok((id, outcome))
}

fn poll(session: &mut Session<'_>, id: usize) -> Result<StepResult, OperationError> {
    match session.take(id)? {
        Acquired::Pending(future) => poll_pending(session, id, future),
        other @ Acquired::Granted(_) => {
            Err(session.reject(id, other, "poll names an acquisition that is not waiting"))
        }
    }
}

fn poll_pending<'limiter>(
    session: &mut Session<'limiter>,
    id: usize,
    mut future: Pin<Box<dyn Future<Output = Result<Permit, AcquireError>> + 'limiter>>,
) -> Result<StepResult, OperationError> {
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(Ok(permit)) => {
            session.put(id, Acquired::Granted(permit));
            Ok(StepResult::Poll {
                id,
                outcome: PollOutcome::Granted,
                state: session.snapshot(),
            })
        }
        Poll::Ready(Err(AcquireError::DeadlineExceeded)) => Ok(StepResult::Poll {
            id,
            outcome: PollOutcome::TimedOut,
            state: session.snapshot(),
        }),
        Poll::Ready(Err(AcquireError::Overloaded)) => Err(OperationError::InvalidRequest {
            message: "a queued acquisition cannot become overloaded".to_owned(),
        }),
        Poll::Pending => {
            session.put(id, Acquired::Pending(future));
            Ok(StepResult::Poll {
                id,
                outcome: PollOutcome::Queued,
                state: session.snapshot(),
            })
        }
    }
}

fn withdraw(session: &mut Session<'_>, request: Withdraw) -> Result<StepResult, OperationError> {
    let id = request.id();
    if let Withdraw::Release { .. } = request {
        match session.take(id)? {
            Acquired::Granted(permit) => {
                drop(permit);
                return Ok(StepResult::Release {
                    id,
                    state: session.snapshot(),
                });
            }
            other @ Acquired::Pending(_) => {
                return Err(session.reject(id, other, request.rejection()));
            }
        }
    }
    match session.take(id)? {
        Acquired::Pending(future) => {
            drop(future);
            Ok(StepResult::Cancel {
                id,
                state: session.snapshot(),
            })
        }
        other @ Acquired::Granted(_) => Err(session.reject(id, other, request.rejection())),
    }
}

fn fill(
    session: &mut Session<'_>,
    count: u64,
    deadline_millis: u64,
) -> Result<StepResult, OperationError> {
    if count > MAX_FILL {
        return Err(OperationError::InvalidRequest {
            message: "a fill is limited to 4096 acquisitions".to_owned(),
        });
    }
    let mut counts = FillCounts::default();
    for _ in 0..count {
        counts.add(start(session, deadline_millis)?.1);
    }
    Ok(StepResult::Fill {
        queued: counts.queued,
        overloaded: counts.overloaded,
        granted: counts.granted,
        timed_out: counts.timed_out,
        state: session.snapshot(),
    })
}

fn deadline(millis: u64) -> Result<Instant, OperationError> {
    Instant::now()
        .checked_add(Duration::from_millis(millis))
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "the deadline is too far in the future".to_owned(),
        })
}
