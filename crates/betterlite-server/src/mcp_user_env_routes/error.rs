use std::collections::BTreeMap;
use std::error::Error as StdError;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_db::mcp_user_env_vars::error::UserEnvVarsError;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_mcp::registry::resolution::{ResolveError, ServerAccessCheck};
use betterlite_models::decisions::Json as JsonValue;
use serde::Serialize;
use thiserror::Error;

use crate::mcp::management_server_access::AccessibleServerSources;
use crate::proxy_helpers::checks::ProxyCheckError;

use super::body::BodyRefusal;
use super::status::StatusError;

pub(crate) const DATABASE_NOT_CONNECTED: &str =
    "Database not connected. Connect a database to your proxy";
const USER_ID_MISSING: &str = "User ID not found in token";
const INTERNAL_MESSAGE: &str = "Internal server error";
const INTERNAL_TYPE: &str = "internal_server_error";

pub(crate) type RouteError<Access, Sources> = UserEnvRouteError<
    <Access as ServerAccessCheck>::Error,
    <Sources as AccessibleServerSources>::Error,
>;

// A FastAPI HTTPException: the status it raises and the object under "detail"
#[derive(Debug)]
pub(crate) struct HttpRefusal {
    status: StatusCode,
    detail: BTreeMap<String, JsonValue>,
}

impl HttpRefusal {
    pub(crate) fn missing_user_id() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            detail: BTreeMap::from([(
                "error".to_owned(),
                JsonValue::Text(USER_ID_MISSING.to_owned()),
            )]),
        }
    }

    pub(crate) fn from_database(error: &ProxyCheckError) -> Self {
        let detail = error.detail().unwrap_or_else(|| {
            BTreeMap::from([("error".to_owned(), JsonValue::Text(error.to_string()))])
        });
        Self {
            status: error.status_code(),
            detail,
        }
    }

    pub(crate) fn from_detail(status: StatusCode, detail: BTreeMap<String, String>) -> Self {
        Self {
            status,
            detail: detail
                .into_iter()
                .map(|(name, text)| (name, JsonValue::Text(text)))
                .collect(),
        }
    }

    fn message(&self) -> String {
        match self.detail.get("error") {
            Some(JsonValue::Text(text)) => text.clone(),
            Some(_) | None => self
                .status
                .canonical_reason()
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum UserEnvRouteError<AccessFailure, SourcesFailure>
where
    AccessFailure: StdError + Send + Sync + 'static,
    SourcesFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Body(#[from] BodyRefusal),
    #[error("{}", .0.message())]
    Refusal(HttpRefusal),
    #[error("the server env vars could not be read as JSON")]
    EnvVars(#[from] StatusError),
    #[error("the per-user env vars could not be read or written")]
    Store(#[from] UserEnvVarsError),
    #[error("the MCP server could not be resolved")]
    Lookup(#[source] ResolveError),
    #[error("the MCP server access check failed")]
    Access(#[source] AccessFailure),
    #[error("the accessible MCP servers could not be listed")]
    Sources(#[source] SourcesFailure),
    #[error("the management default auth could not be built")]
    DefaultAuth(#[source] AuthError),
}

#[derive(Serialize)]
struct DetailBody {
    detail: BTreeMap<String, JsonValue>,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

impl<AccessFailure, SourcesFailure> UserEnvRouteError<AccessFailure, SourcesFailure>
where
    AccessFailure: StdError + Send + Sync + 'static,
    SourcesFailure: StdError + Send + Sync + 'static,
{
    // What a failure span receives in LiteLLM's place: the class, the status and a message with no value in it
    pub(crate) fn facts(&self) -> ExceptionFacts {
        let (class_name, status) = match self {
            Self::Body(refusal) => (refusal.class_name(), Some(refusal.status())),
            Self::Refusal(refusal) => ("HTTPException", Some(refusal.status)),
            Self::EnvVars(_)
            | Self::Store(_)
            | Self::Lookup(_)
            | Self::Access(_)
            | Self::Sources(_)
            | Self::DefaultAuth(_) => ("Exception", None),
        };
        let status_code = status.as_ref().map(StatusCode::as_u16);
        ExceptionFacts {
            class_name: class_name.to_owned(),
            class: None,
            message: Some(self.to_string()),
            code: None,
            status_code,
            response_status_code: status_code,
            llm_provider: None,
            category: None,
            rate_limit_type: None,
            budget_entity_type: None,
            budget_entity_id: None,
            budget_limit: None,
            budget_spend: None,
            proxy_type: None,
            headers: None,
            response_headers: None,
            litellm_response_headers: None,
            traceback_lines: Vec::new(),
        }
    }
}

impl<AccessFailure, SourcesFailure> IntoResponse
    for UserEnvRouteError<AccessFailure, SourcesFailure>
where
    AccessFailure: StdError + Send + Sync + 'static,
    SourcesFailure: StdError + Send + Sync + 'static,
{
    fn into_response(self) -> Response {
        match self {
            Self::Body(refusal) => refusal.into_response(),
            Self::Refusal(refusal) => (
                refusal.status,
                Json(DetailBody {
                    detail: refusal.detail,
                }),
            )
                .into_response(),
            internal @ (Self::EnvVars(_)
            | Self::Store(_)
            | Self::Lookup(_)
            | Self::Access(_)
            | Self::Sources(_)
            | Self::DefaultAuth(_)) => {
                tracing::error!(error = %internal, "the MCP user env vars request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(InternalBody {
                        error: InternalMessage {
                            message: INTERNAL_MESSAGE,
                            error_type: INTERNAL_TYPE,
                        },
                    }),
                )
                    .into_response()
            }
        }
    }
}
