use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use serde::Deserialize;

use super::super::state::InvitationRouteState;
use super::super::{Caller, InvitationRouteError};
use super::RouteResult;
use crate::search_tool_routes::body::decode_body;
use crate::user_invitation::InvitationRecord;

#[derive(Deserialize)]
struct AcceptInvitation {
    invitation_id: String,
    is_accepted: bool,
}

pub(crate) async fn update_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> RouteResult {
    let Some(Extension(context)) = caller else {
        return Err(InvitationRouteError::Unauthenticated);
    };
    let request: AcceptInvitation = decode_body(&body).map_err(InvitationRouteError::Body)?;
    let record = accept_invitation(&state, &context.principal, &request).await?;
    Ok(Json(record).into_response())
}

async fn accept_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    request: &AcceptInvitation,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state.database()?;
    let Some(user_id) = principal.user.as_ref().map(|user| user.id.as_str()) else {
        return Err(InvitationRouteError::UserUnidentified);
    };
    database
        .invitations
        .update(&request.invitation_id, request.is_accepted, user_id)
        .await
        .map_err(InvitationRouteError::Store)?
        .ok_or(InvitationRouteError::InvitationMissing)
}
