use crate::state::ServerState;
use axum::Router;
use std::sync::Arc;

#[cfg_attr(test, mutants::skip)]
pub(crate) fn routes(_state: &Arc<ServerState>) -> Router {
    // the route owner fills this router
    Router::new()
}
