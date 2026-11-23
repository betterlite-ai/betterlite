use std::sync::Arc;

use axum::Router;
use betterlite_core::api::api_boundaries::ApiLimits;

use crate::api::claude_plugin_write::{ClaudePluginWriteState, database_store, mount};
use crate::mcp::outbound_credentials::SystemWallClock;
use crate::state::ServerState;

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let write = ClaudePluginWriteState {
        store: database_store(state),
        clock: Arc::new(SystemWallClock),
        limits: ApiLimits::default(),
    };
    mount(write)
}
