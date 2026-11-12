use crate::state::ServerState;
use axum::Router;
use std::sync::Arc;

#[must_use]
pub(crate) fn routes(_state: &Arc<ServerState>) -> Router {
    Router::new()
}
