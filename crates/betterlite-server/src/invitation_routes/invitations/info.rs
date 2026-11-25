use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;
use serde::Deserialize;

use super::super::state::InvitationRouteState;
use super::super::{Caller, InvitationRouteError};
use super::{MASTER_KEY_SUBJECT, RouteResult, role_text};
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
    let admin_view = principal.subject == MASTER_KEY_SUBJECT
        || principal
            .source_key
            .0
            .as_ref()
            .is_some_and(user_api_key_has_admin_view)
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer);
    if !admin_view {
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
