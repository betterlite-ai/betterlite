mod fallback_login;
mod invitations;
mod queue;
pub mod state;
pub mod store;
mod tests;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use betterlite_config::sections::login_form::Settings;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;

use self::state::{InvitationDatabase, InvitationRouteState};
use self::store::InvitationStoreError;
use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;
use crate::login_form::{LoginForm, should_hide_credentials_hint};
use crate::search_tool_routes::body::BodyError;
use crate::tracing_routes::router::unauthenticated;

const INVITATION_ID: &str = "invitation_id";
const FIELD_REQUIRED: &str = "Field required";

pub(crate) type Caller = Option<Extension<AuthContext>>;

// B799's login page, rendered without the deprecation banner that the /login route adds
#[derive(Debug, Clone)]
pub struct LoginPageState {
    pub form: LoginForm,
    pub hide_default_credentials_hint: bool,
}

impl LoginPageState {
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            form: LoginForm::from_settings(settings),
            hide_default_credentials_hint: should_hide_credentials_hint(settings),
        }
    }
}

// The invitation routes and the queued chat route, mounted inside the auth layers
pub fn routes(state: InvitationRouteState) -> Router {
    Router::new()
        .route("/invitation/new", post(invitations::new_invitation))
        .route("/invitation/info", get(invitations::invitation_info))
        .route("/invitation/update", post(invitations::update_invitation))
        .route("/invitation/delete", post(invitations::delete_invitation))
        .route(
            "/queue/chat/completions",
            post(queue::queue_chat_completions),
        )
        .with_state(Arc::new(state))
}

// The routes the proxy answers without a key, mounted outside the auth layers
pub fn public_routes(login: LoginPageState) -> Router {
    Router::new()
        .route("/fallback/login", get(fallback_login::fallback_login))
        .with_state(Arc::new(login))
}

impl InvitationRouteState {
    const fn database(&self) -> Result<&InvitationDatabase, InvitationRouteError> {
        match self.invitations.as_ref() {
            Some(database) => Ok(database),
            None => Err(InvitationRouteError::Database),
        }
    }
}

#[derive(Debug, Error)]
pub enum InvitationRouteError {
    // BodyError carries the FastAPI validation body itself and has no Display of its own
    #[error("the request body is not a valid invitation request")]
    Body(BodyError),
    #[error("the query parameter invitation_id is required")]
    MissingInvitationId,
    #[error("the database is not connected")]
    Database,
    #[error("the caller is neither a proxy admin nor an admin of its organization or team")]
    NotAllowed { role: &'static str },
    #[error("You can only create invitations for users in your organization or team.")]
    InviteNotAllowed,
    #[error("Invitation id does not exist in the database.")]
    InvitationMissing,
    #[error("Unable to identify user id. Received=None")]
    UserUnidentified,
    #[error("Organization admins can only delete invitations they created.")]
    OrgAdminDelete,
    #[error("a key is required")]
    Unauthenticated,
    #[error(transparent)]
    Store(InvitationStoreError),
}

impl InvitationRouteError {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::MissingInvitationId => FIELD_REQUIRED.to_owned(),
            Self::Database => CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
            Self::NotAllowed { role } => format!(
                "{}, your role={role}",
                CommonProxyErrors::NotAllowedAccess.as_str()
            ),
            Self::Store(error) => error.message(),
            other @ (Self::Body(_)
            | Self::InviteNotAllowed
            | Self::InvitationMissing
            | Self::UserUnidentified
            | Self::OrgAdminDelete
            | Self::Unauthenticated) => other.to_string(),
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Body(_) | Self::MissingInvitationId => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Database
            | Self::NotAllowed { .. }
            | Self::InviteNotAllowed
            | Self::InvitationMissing => StatusCode::BAD_REQUEST,
            Self::UserUnidentified => StatusCode::INTERNAL_SERVER_ERROR,
            Self::OrgAdminDelete => StatusCode::FORBIDDEN,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Store(error) => error.status(),
        }
    }
}

impl IntoResponse for InvitationRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Body(error) => error.into_response(),
            Self::MissingInvitationId => missing_query_parameter(),
            Self::Unauthenticated => unauthenticated(),
            Self::Store(error) => {
                tracing::error!(error = %error, "the invitation store request failed");
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                    "internal_server_error",
                    "500",
                )
            }
            other @ (Self::Database
            | Self::NotAllowed { .. }
            | Self::InviteNotAllowed
            | Self::InvitationMissing
            | Self::UserUnidentified
            | Self::OrgAdminDelete) => detail_response(other.status(), other.message()),
        }
    }
}

#[derive(Serialize)]
struct DetailBody {
    detail: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    error: String,
}

#[derive(Serialize)]
struct QueryBody {
    detail: [QueryIssue; 1],
}

#[derive(Serialize)]
struct QueryIssue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

fn detail_response(status: StatusCode, message: String) -> Response {
    let body = DetailBody {
        detail: ErrorDetail { error: message },
    };
    (status, Json(body)).into_response()
}

// FastAPI rejects a missing required query parameter before the handler runs
fn missing_query_parameter() -> Response {
    let issue = QueryIssue {
        issue_type: "missing",
        loc: ["query", INVITATION_ID],
        msg: FIELD_REQUIRED,
    };
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(QueryBody { detail: [issue] }),
    )
        .into_response()
}
