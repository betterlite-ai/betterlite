use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use thiserror::Error;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::task::TaskTracker;

const WAITING: u8 = 0;
const COMMITTING: u8 = 1;
const CANCELLED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitPhaseState {
    Waiting,
    Committing,
    Cancelled,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CommitPhaseError {
    #[error("the daily spend commit phase is already committing")]
    AlreadyCommitting,
    #[error("the daily spend commit phase is not committing")]
    NotCommitting,
    #[error("the daily spend commit was cancelled before it began")]
    Cancelled,
}

// The source's event covers three state changes: entering the commit, leaving it, and the shutdown's cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitTransition {
    Begin,
    Finish,
    Cancel,
}

// The source's per-chunk Event: the moment between execute_raw returning and the transaction leaving.
pub struct CommitPhase {
    state: AtomicU8,
}

impl CommitPhase {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(WAITING),
        }
    }

    #[must_use]
    pub fn state(&self) -> CommitPhaseState {
        match self.state.load(Ordering::SeqCst) {
            COMMITTING => CommitPhaseState::Committing,
            CANCELLED => CommitPhaseState::Cancelled,
            // the phase only ever stores the three constants above, so any other byte is the untouched initial state
            _ => CommitPhaseState::Waiting,
        }
    }

    // _mark_daily_spend_commit_started and _mark_daily_spend_commit_finished: entering the commit and cancelling it are compare-and-sets on the same byte, so the two can never both win
    pub fn apply(&self, transition: CommitTransition) -> Result<(), CommitPhaseError> {
        let (from, to) = match transition {
            CommitTransition::Begin => (WAITING, COMMITTING),
            CommitTransition::Finish => (COMMITTING, WAITING),
            CommitTransition::Cancel => (WAITING, CANCELLED),
        };
        match self
            .state
            .compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_previous) => Ok(()),
            Err(COMMITTING) => Err(CommitPhaseError::AlreadyCommitting),
            Err(CANCELLED) => Err(CommitPhaseError::Cancelled),
            Err(_other) => Err(CommitPhaseError::NotCommitting),
        }
    }
}

impl Default for CommitPhase {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StartedDailyCommit<T> {
    pub phase: Arc<CommitPhase>,
    pub task: JoinHandle<T>,
}

#[derive(Debug)]
pub enum DailyCommitSettlement<T> {
    Cancelled,
    Settled(Result<T, JoinError>),
}

// _start_daily_spend_commit: the queue's own tracker owns the commit, and the phase it marks is handed back with it
#[must_use]
pub fn start_daily_spend_commit<T, Run, Work>(
    commits: &TaskTracker,
    run: Run,
) -> StartedDailyCommit<T>
where
    Run: FnOnce(Arc<CommitPhase>) -> Work,
    Work: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let phase = Arc::new(CommitPhase::new());
    let task = commits.spawn(run(Arc::clone(&phase)));
    StartedDailyCommit { phase, task }
}

impl<T> StartedDailyCommit<T> {
    // The shutdown path cancels work that never reached its commit, and owns the rest until its definitive result.
    pub async fn settle_for_shutdown(self) -> DailyCommitSettlement<T> {
        match self.phase.apply(CommitTransition::Cancel) {
            Ok(()) => {
                self.task.abort();
                DailyCommitSettlement::Cancelled
            }
            Err(_committing) => DailyCommitSettlement::Settled(self.task.await),
        }
    }
}
