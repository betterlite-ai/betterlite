pub mod capped;
pub mod dates;
pub mod error;
pub mod filters;
mod legacy;
pub mod projection;
pub mod query;
pub mod rows;
pub mod session;
pub mod store;
pub mod summary;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use self::store::LogResultsStore;

#[derive(Clone)]
pub struct LogResultsState {
    // None when the proxy runs without a database; the legacy listing then answers LiteLLM's no-database error
    pub store: Option<Arc<dyn LogResultsStore>>,
}

pub fn routes(state: LogResultsState) -> Router {
    Router::new()
        .route("/spend/logs", get(legacy::view_spend_logs))
        .with_state(state)
}
