use std::sync::Arc;

use axum::Router;
use betterlite_core::api::api_boundaries::ApiLimits;

use crate::api::claude_plugin_read::{ClaudePluginReadState, database_store, mount};
use crate::state::ServerState;

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    let read = ClaudePluginReadState {
        store: database_store(state),
        limits: ApiLimits::default(),
    };
    mount(read)
}
