mod access;
mod error;
mod flow;
mod store;
mod wire;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, DB_NOT_CONNECTED, require_database,
};
use betterlite_core::api::claude_plugin_read::PluginManifestFields;
use betterlite_core::api::claude_plugin_read::error::PluginManifestError;
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::app::spend::cli_writer_store;
use crate::state::ServerState;

use self::store::{ClaudePluginReadStore, PostgresPluginReadStore};

pub(crate) const PLUGINS_PATH: &str = "/claude-code/plugins";
const PLUGIN_PATH: &str = "/claude-code/plugins/{plugin_name}";

pub(crate) struct ClaudePluginReadState {
    pub(crate) store: Option<Arc<dyn ClaudePluginReadStore>>,
    pub(crate) limits: ApiLimits,
}

#[derive(Debug, Error)]
pub enum ClaudePluginReadDecodeError {
    #[error("the stored plugin manifest is {observed} bytes; the maximum is {limit}")]
    TooLarge { observed: usize, limit: usize },
    #[error("the stored plugin manifest is not valid JSON")]
    Invalid {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Shape(#[from] PluginManifestError),
}

impl ClaudePluginReadState {
    // _get_prisma_client: both reads need the database, and its 500 carries the pinned detail
    fn connected_store(&self) -> Result<Arc<dyn ClaudePluginReadStore>, ClaudePluginPolicyError> {
        require_database(self.store.is_some())?;
        self.store
            .clone()
            .ok_or(ClaudePluginPolicyError::DatabaseNotConnected {
                message: DB_NOT_CONNECTED,
            })
    }
}

pub(crate) fn mount(state: ClaudePluginReadState) -> Router {
    Router::new()
        .route(PLUGINS_PATH, get(flow::read))
        .route(PLUGIN_PATH, get(flow::read))
        .with_state(Arc::new(state))
}

pub(crate) fn database_store(state: &Arc<ServerState>) -> Option<Arc<dyn ClaudePluginReadStore>> {
    let store: Arc<dyn ClaudePluginReadStore> =
        Arc::new(PostgresPluginReadStore::new(cli_writer_store(state)?));
    Some(store)
}

// json.loads(p.manifest_json) at list_plugins and json.loads(plugin.manifest_json or "{}") at get_plugin
pub fn decode_claude_plugin_read(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<PluginManifestFields, ClaudePluginReadDecodeError> {
    let boundary = ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    });
    if limits.check(boundary).is_err() {
        return Err(ClaudePluginReadDecodeError::TooLarge {
            observed: bytes.len(),
            limit: limits.max_json_bytes,
        });
    }
    let manifest: Json = serde_json::from_slice(bytes)
        .map_err(|source| ClaudePluginReadDecodeError::Invalid { source })?;
    Ok(PluginManifestFields::from_manifest(manifest)?)
}
