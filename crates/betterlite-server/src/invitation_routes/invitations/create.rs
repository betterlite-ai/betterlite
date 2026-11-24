use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use serde::Deserialize;

use super::super::state::{InvitationDatabase, InvitationRouteState};
use super::super::{Caller, InvitationRouteError};
use super::{RouteResult, role_text};
use crate::api::agents::collection_routes::guards::is_proxy_admin;
use crate::auth::auth_resource_grants::denial_response;
use crate::search_tool_routes::body::decode_body;
use crate::user_invitation::InvitationRecord;

#[derive(Deserialize)]
struct NewInvitation {
    user_id: String,
}

pub(crate) async fn new_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> RouteResult {
    let Some(Extension(context)) = caller else {
        return Err(InvitationRouteError::Unauthenticated);
    };
    let request: NewInvitation = decode_body(&body).map_err(InvitationRouteError::Body)?;
    match create_invitation(&state, &context.principal, &request).await {
        Ok(record) => Ok(Json(record).into_response()),
        Err(error) => Ok(denial_response(&wrapped(&error))),
    }
}

async fn create_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    request: &NewInvitation,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state.database()?;
    admit_inviter(database, principal, &request.user_id).await?;
    database
        .invitations
        .create(
            &request.user_id,
            principal.user.as_ref().map(|user| user.id.as_str()),
        )
        .await
        .map_err(InvitationRouteError::Store)
}

// PROXY_ADMIN passes; anyone else needs the org-admin check and then the check for the target user
async fn admit_inviter(
    database: &InvitationDatabase,
    principal: &Principal,
    user_id: &str,
) -> Result<(), InvitationRouteError> {
    if is_proxy_admin(principal) {
        return Ok(());
    }
    if !database.scope.has_admin_privileges(principal).await {
        return Err(InvitationRouteError::NotAllowed {
            role: role_text(principal),
        });
    }
    if database.scope.can_invite_user(principal, user_id).await {
        return Ok(());
    }
    Err(InvitationRouteError::InviteNotAllowed)
}

// LiteLLM's /invitation/new raises through handle_exception_on_proxy, which turns every failure into an internal_server_error ProxyException
fn wrapped(error: &InvitationRouteError) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: error.message(),
        error_type: ProxyErrorTypes::InternalServerError.to_string(),
        param: None,
        code: ExceptionCode::Status(i64::from(error.status().as_u16())),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
