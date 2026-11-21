use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::MCPAuth;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::server_record::MCPServer;

#[derive(Clone, Copy, Debug, Eq, PartialEq, derive_more::Display)]
pub enum FailureReason {
    #[display("timed out")]
    TimedOut,
    #[display("returned incomplete metadata")]
    Incomplete,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SlotOutcome {
    Resolved(Box<MCPServer>),
    Failed(FailureReason),
    Stale,
}

pub type OutcomeWatch = watch::Receiver<Option<SlotOutcome>>;

pub(super) struct DiscoveryRun {
    pub(super) outcome: OutcomeWatch,
    pub(super) abort: AbortHandle,
    // what the run discovers for, so reconcile can drop it once the server moves
    pub(super) url: Option<String>,
    pub(super) auth_type: Option<MCPAuth>,
}

pub(super) struct DiscoverySlot {
    pub(super) generation: u64,
    pub(super) consecutive_failures: u32,
    pub(super) retry_after: Instant,
    pub(super) expires: Option<Instant>,
    pub(super) run: Option<DiscoveryRun>,
}

#[derive(Default)]
pub(super) struct SlotTable {
    pub(super) generation: u64,
    pub(super) slots: BTreeMap<String, DiscoverySlot>,
}

impl SlotTable {
    pub(super) fn sweep(&mut self, now: Instant) {
        self.slots
            .retain(|_, slot| slot.expires.is_none_or(|expires| now < expires));
    }
}
