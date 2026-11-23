pub mod baseline_capture;
pub mod discovery;
pub mod enqueue;
pub mod flush;
pub mod payload;
pub mod queues;
pub mod turn_enqueue;

use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::baseline::flush::queue::BaselineQueue;
use crate::daily_records::projection::{CallTypeEndpoints, DailyPricing};
use crate::spend_log_storage::rows::SpendLogQueue;

use self::queues::UsageQueue;

// get_llm_router: the proxy's pricing catalog and calendar arrive through the caller, never an ambient import.
pub struct TurnEnvironment<'run, C> {
    pub pricing: &'run DailyPricing<'run, C>,
    pub endpoints: &'run dyn CallTypeEndpoints,
}

// The request-time queues one auto-router turn enqueue writes to; None stands for a run with no database client.
pub struct AutorouterQueues<'run> {
    pub turns: &'run UsageQueue<AutoRouterTurnTransaction>,
    pub baseline: &'run BaselineQueue,
    pub logs: &'run SpendLogQueue,
}
