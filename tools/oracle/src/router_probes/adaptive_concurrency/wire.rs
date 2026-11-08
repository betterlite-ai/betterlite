use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Snapshot,
    Record { throttled: bool },
    Acquire { deadline_millis: u64 },
    Poll { id: usize },
    Release { id: usize },
    Cancel { id: usize },
    Fill { count: u64, deadline_millis: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSnapshot {
    pub limit: u64,
    pub in_flight: u64,
    pub waiters: usize,
    pub clean_streak: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquireOutcome {
    Granted,
    Queued,
    Overloaded,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollOutcome {
    Granted,
    Queued,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepResult {
    Snapshot {
        state: StateSnapshot,
    },
    Record {
        state: StateSnapshot,
    },
    Acquire {
        id: usize,
        outcome: AcquireOutcome,
        state: StateSnapshot,
    },
    Poll {
        id: usize,
        outcome: PollOutcome,
        state: StateSnapshot,
    },
    Release {
        id: usize,
        state: StateSnapshot,
    },
    Cancel {
        id: usize,
        state: StateSnapshot,
    },
    Fill {
        queued: u64,
        overloaded: u64,
        granted: u64,
        timed_out: u64,
        state: StateSnapshot,
    },
}
