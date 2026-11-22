use crate::state::ServerState;
use axum::Router;
use std::sync::Arc;

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    crate::api::claude_plugin_write::routes(state)
}
