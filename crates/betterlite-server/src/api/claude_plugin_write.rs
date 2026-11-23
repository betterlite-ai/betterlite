mod access;
mod error;
mod flow;
mod lifecycle;
mod store;
mod wire;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Extension};
use axum::routing::{post, put};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, DB_NOT_CONNECTED, require_database,
};

use crate::app::spend::cli_writer_store;
use crate::mcp::outbound_credentials::WallClock;
use crate::state::ServerState;

use self::lifecycle::PluginToggle;
use self::store::{ClaudePluginStore, PostgresPluginStore};

pub(crate) const PLUGINS_PATH: &str = "/claude-code/plugins";
const PLUGIN_PATH: &str = "/claude-code/plugins/{plugin_name}";
const PLUGIN_ENABLE_PATH: &str = "/claude-code/plugins/{plugin_name}/enable";
const PLUGIN_DISABLE_PATH: &str = "/claude-code/plugins/{plugin_name}/disable";

pub(crate) struct ClaudePluginWriteState {
    pub(crate) store: Option<Arc<dyn ClaudePluginStore>>,
    pub(crate) clock: Arc<dyn WallClock>,
    pub(crate) limits: ApiLimits,
}

impl ClaudePluginWriteState {
    // _get_prisma_client: every mutation needs the database, and its 500 carries the pinned detail
    fn connected_store(&self) -> Result<Arc<dyn ClaudePluginStore>, ClaudePluginPolicyError> {
        require_database(self.store.is_some())?;
        self.store
            .clone()
            .ok_or(ClaudePluginPolicyError::DatabaseNotConnected {
                message: DB_NOT_CONNECTED,
            })
    }
}

pub(crate) fn mount(state: ClaudePluginWriteState) -> Router {
    Router::new()
        .route(PLUGINS_PATH, post(flow::create))
        .route(PLUGIN_PATH, put(flow::update).delete(lifecycle::remove))
        .route(
            PLUGIN_ENABLE_PATH,
            post(lifecycle::toggle).layer(Extension(PluginToggle::Enable)),
        )
        .route(
            PLUGIN_DISABLE_PATH,
            post(lifecycle::toggle).layer(Extension(PluginToggle::Disable)),
        )
        // B1087's ApiLimits owns the JSON limit; axum's 2 MB extractor default would answer first
        .layer(DefaultBodyLimit::disable())
        .with_state(Arc::new(state))
}

pub(crate) fn database_store(state: &Arc<ServerState>) -> Option<Arc<dyn ClaudePluginStore>> {
    let store: Arc<dyn ClaudePluginStore> =
        Arc::new(PostgresPluginStore::new(cli_writer_store(state)?));
    Some(store)
}
