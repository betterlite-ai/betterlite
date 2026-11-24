pub mod cursor;
pub mod error;
pub mod expansion;
pub mod grouped;
pub mod params;
pub mod ports;
pub mod query;
pub mod rows;
pub mod store;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use self::expansion::SessionTeamScope;
use self::ports::SessionsStore;

// None when the proxy runs without a database; the expansion route then answers "Database not connected"
#[derive(Clone)]
pub struct SessionsState {
    pub store: Option<Arc<dyn SessionsStore>>,
    // B1704's permitted-team lookup; a missing or failing lookup narrows a caller to their own rows
    pub scope: Option<Arc<dyn SessionTeamScope>>,
}

pub fn routes(state: SessionsState) -> Router {
    Router::new()
        .route(
            "/spend/logs/session/ui",
            get(expansion::ui_view_session_spend_logs),
        )
        .with_state(state)
}
