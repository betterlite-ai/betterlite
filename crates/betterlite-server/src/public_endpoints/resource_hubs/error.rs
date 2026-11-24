use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_core::api::claude_plugin_policy::ClaudePluginPolicyError;
use betterlite_db::auxiliary_repository::repository::TableError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResourceHubsError {
    #[error("the agent card is not valid")]
    AgentCard(#[source] serde_json::Error),
    #[error("the MCP server metadata is not valid")]
    McpInfo(#[source] serde_json::Error),
    #[error("the plugin manifest is not a JSON object")]
    Manifest(#[source] serde_json::Error),
    #[error("the plugin manifest field `source` must be an object")]
    ManifestSource,
    #[error("the plugin table could not be read")]
    PluginTable(#[source] TableError),
    #[error(transparent)]
    PluginPolicy(ClaudePluginPolicyError),
}

// the skill route's except stringifies even its own HTTPException, so failures answer a string detail
#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl IntoResponse for ResourceHubsError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self, "public resource hub failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(DetailBody {
                detail: self.to_string(),
            }),
        )
            .into_response()
    }
}
