pub mod authz;
pub mod conditions;
pub mod error;
pub mod execute;
pub mod handler;
pub mod order;
pub mod page;
pub mod scope;
pub mod store;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;

use self::scope::LogListScope;
use self::store::LogListStore;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::log_results::session::SessionAggregateReader;
use crate::spend::sessions::ports::SessionsStore;

// every handle is absent when the proxy runs without a database, and the routes then answer LiteLLM's missing-database error
#[derive(Clone)]
pub struct LogListState {
    pub store: Option<Arc<dyn LogListStore>>,
    pub sessions: Option<Arc<dyn SessionsStore>>,
    pub scope: Option<Arc<dyn LogListScope>>,
    pub enricher: Option<Arc<dyn SessionAggregateReader>>,
}

// GET /spend/logs/v2 (public) and GET /spend/logs/ui (internal) share one handler, told apart by the route it is layered with
pub fn routes(state: LogListState) -> Router {
    Router::new()
        .route(
            "/spend/logs/ui",
            get(handler::ui_view_spend_logs).layer(Extension(SpendLogRoute::Ui)),
        )
        .route(
            "/spend/logs/v2",
            get(handler::ui_view_spend_logs).layer(Extension(SpendLogRoute::PublicV2)),
        )
        .with_state(state)
}
