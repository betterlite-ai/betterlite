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
            Self::Body(_) | Self::Request(_) => StatusCode::UNPROCESSABLE_ENTITY,
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
}

const JSON_DECODE_ERROR: &str = "JSON decode error";

// FastAPI's RequestValidationError sends only type, loc and msg, without input or ctx
#[derive(Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: Vec<&'static str>,
    msg: String,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

fn validation_response(status: StatusCode, kind: &'static str, msg: String) -> Response {
    let body = ValidationBody {
        detail: vec![ValidationIssue {
            kind,
            loc: vec!["body"],
            msg,
        }],
    };
    (status, Json(body)).into_response()
}

// the HTTPException detail LiteLLM raises for everything but a body-validation failure
#[derive(Serialize)]
struct DetailBody {
    detail: BTreeMap<String, String>,
}

impl<Failure> IntoResponse for SessionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    fn into_response(self) -> Response {
        if matches!(self, Self::Unauthenticated) {
            return unauthenticated();
        }
        let status = self.status();
        match self {
            Self::Body(_) => {
                validation_response(status, "json_invalid", JSON_DECODE_ERROR.to_owned())
            }
            Self::Request(error) => {
                validation_response(status, "value_error", format!("Value error, {error}"))
            }
            Self::NotFound(detail) | Self::Forbidden(detail) => {
                (status, Json(DetailBody { detail })).into_response()
            }
            other @ (Self::Payload(_)
            | Self::Record(_)
            | Self::Table(_)
            | Self::Write(_)
            | Self::Build(_)
            | Self::Resolve(_)
            | Self::CredentialView(_)
            | Self::Access(_)
            | Self::MissingRuntime { .. }
            | Self::Caching { .. }
            | Self::Unauthenticated) => (
                status,
                Json(DetailBody {
                    detail: BTreeMap::from([("error".to_owned(), other.to_string())]),
                }),
            )
                .into_response(),
        }
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
