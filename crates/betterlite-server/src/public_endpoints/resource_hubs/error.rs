use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
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

#[derive(Serialize)]
struct ErrorDetail {
    error: String,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

// FastAPI renders HTTPException(detail={...}) as {"detail": {...}}, the body LiteLLM's _get_prisma_client produces.
#[derive(Serialize)]
struct PolicyBody {
    detail: ErrorDetail,
}

impl IntoResponse for ResourceHubsError {
    fn into_response(self) -> Response {
        if let Self::PluginPolicy(policy) = &self {
            let fields = policy.fields();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(PolicyBody {
                    detail: ErrorDetail {
                        error: fields.detail.error,
                    },
                }),
            )
                .into_response();
        }
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
