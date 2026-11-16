pub mod access;
pub mod error;
pub mod models;
pub mod router;
pub mod sequence;
pub mod store;

mod tests;

use std::sync::Arc;

use crate::workflow_runs::store::WorkflowRunStore;

#[derive(Clone)]
pub struct WorkflowRunsState {
    pub store: Option<Arc<dyn WorkflowRunStore>>,
}
