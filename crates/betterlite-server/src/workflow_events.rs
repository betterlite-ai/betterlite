pub mod error;
pub mod events;
pub mod messages;
pub mod models;
pub mod read;
pub mod router;
pub mod store;
pub mod write;

mod tests;

use std::sync::Arc;

use crate::workflow_events::store::WorkflowEventStore;
use crate::workflow_runs::store::WorkflowRunStore;

// Both ports read the same database; either one missing means it is not connected.
#[derive(Clone)]
pub struct WorkflowEventsState {
    pub runs: Option<Arc<dyn WorkflowRunStore>>,
    pub events: Option<Arc<dyn WorkflowEventStore>>,
}
