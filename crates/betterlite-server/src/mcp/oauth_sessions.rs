pub mod record;
pub mod redis_copy;
pub mod route;
pub mod store;

mod tests;

use std::collections::BTreeMap;
use std::error::Error as StdError;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_mcp::registry::resolution::{AuthorizeError, ResolveError};
use betterlite_mcp::server_build::BuildError;
use betterlite_mcp::server_payload::{PayloadError, RequestError};
use serde::Serialize;
use thiserror::Error;

use self::record::RecordError;
use crate::mcp::admin_credential_view::CredentialViewError;
use crate::mcp::server_writes::ServerWriteError;
use crate::tracing_routes::router::unauthenticated;

// ast-grep-ignore: no-long-names - LiteLLM's TEMPORARY_MCP_SERVER_TTL_SECONDS, which B8622 and B8702 read
pub const TEMPORARY_MCP_SERVER_TTL_SECONDS: i64 = 300;
// ast-grep-ignore: no-long-names - LiteLLM's TEMPORARY_MCP_SERVER_REDIS_KEY_PREFIX, the Redis key prefix
pub const TEMPORARY_MCP_SERVER_REDIS_KEY_PREFIX: &str = "litellm:mcp:temporary_server";
// LiteLLM's LITELLM_PROXY_ADMIN_NAME, the created_by of a caller without a user id
pub const DEFAULT_CREATED_BY: &str = "default_user_id";

// One variant per failure the session store and its route can report, with the route's status mapping
#[derive(Debug, Error)]
pub enum SessionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error("the request body could not be decoded")]
    Body(#[source] serde_json::Error),
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Record(#[from] RecordError),
    #[error(transparent)]
    Table(#[from] McpTableError),
    #[error(transparent)]
    Write(#[from] ServerWriteError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    CredentialView(#[from] CredentialViewError),
    #[error("the MCP server was not found")]
    NotFound(BTreeMap<String, String>),
    #[error("the caller may not access this MCP server")]
    Forbidden(BTreeMap<String, String>),
    #[error("the MCP server access check failed")]
    Access(#[source] Failure),
    #[error("MCP server {server_id} has no runtime record")]
    MissingRuntime { server_id: String },
    #[error("Error caching temporary mcp server: {source}")]
    Caching {
        #[source]
        source: Box<Self>,
    },
    #[error("the caller is not authenticated")]
    Unauthenticated,
}

impl<Failure> SessionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Body(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Request(RequestError::Value(_)) => StatusCode::BAD_REQUEST,
            Self::Request(RequestError::Shape(_)) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Payload(PayloadError::UnknownRequiredFields { .. }) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Payload(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Record(_)
            | Self::Table(_)
            | Self::Write(_)
            | Self::Build(_)
            | Self::Resolve(_)
            | Self::CredentialView(_)
            | Self::Access(_)
            | Self::MissingRuntime { .. }
            | Self::Caching { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // the HTTPException detail LiteLLM's route raises: a fixed map, else {"error": str(e)}
    fn detail(&self) -> BTreeMap<String, String> {
        match self {
            Self::NotFound(detail) | Self::Forbidden(detail) => detail.clone(),
            other => BTreeMap::from([("error".to_owned(), other.to_string())]),
        }
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: BTreeMap<String, String>,
}

impl<Failure> IntoResponse for SessionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    fn into_response(self) -> Response {
        if let Self::Unauthenticated = self {
            return unauthenticated();
        }
        let status = self.status();
        (
            status,
            Json(DetailBody {
                detail: self.detail(),
            }),
        )
            .into_response()
    }
}

impl<Failure> From<AuthorizeError<Failure>> for SessionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    fn from(error: AuthorizeError<Failure>) -> Self {
        match error {
            AuthorizeError::NotFound(detail) => Self::NotFound(detail),
            AuthorizeError::Forbidden(detail) => Self::Forbidden(detail),
            AuthorizeError::Access(source) => Self::Access(source),
        }
    }
}
