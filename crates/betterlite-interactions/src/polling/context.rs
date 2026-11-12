use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use secrecy::SecretString;

#[derive(Clone, Debug)]
pub struct InteractionPollContext<Host> {
    pub interaction_id: String,
    pub custom_llm_provider: String,
    pub host: Host,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub timeout: Duration,
    pub settlement: SettlementFlag,
}

#[derive(Clone, Debug, Default)]
pub struct SettlementFlag {
    pub(crate) settled: Arc<AtomicBool>,
}

impl SettlementFlag {
    // the poll task and the delete path race for the one settlement, so only the first claim wins
    #[must_use]
    pub fn claim_settlement(&self) -> bool {
        self.settled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}
