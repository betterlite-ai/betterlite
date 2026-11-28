use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use serde::Deserialize;

use super::super::state::InvitationRouteState;
use super::super::{Caller, InvitationRouteError};
use super::{RouteResult, role_text};
use crate::management_guards::caller::caller_has_admin_view;
use crate::user_invitation::InvitationRecord;

#[derive(Deserialize)]
pub(crate) struct InfoQuery {
    invitation_id: Option<String>,
}

pub(crate) async fn invitation_info(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    query: Query<InfoQuery>,
) -> RouteResult {
    let Some(Extension(context)) = caller else {
        return Err(InvitationRouteError::Unauthenticated);
    };
    let invitation_id = query
        .invitation_id
        .as_deref()
        .ok_or(InvitationRouteError::MissingInvitationId)?;
    let record = read_invitation(&state, &context.principal, invitation_id).await?;
    Ok(Json(record).into_response())
}

async fn read_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    invitation_id: &str,
) -> Result<InvitationRecord, InvitationRouteError> {
    // LiteLLM checks prisma_client (L17799) before the admin view (L17805)
    let database = state.database()?;
    if !caller_has_admin_view(principal) {
        return Err(InvitationRouteError::NotAllowed {
            role: role_text(principal),
        });
    }
    database
        .invitations
        .info(invitation_id)
        .await
        .map_err(InvitationRouteError::Store)?
        .ok_or(InvitationRouteError::InvitationMissing)
}
