use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde::Deserialize;

use super::super::state::{InvitationDatabase, InvitationRouteState};
use super::super::{Caller, InvitationRouteError};
use super::{RouteResult, role_text};
use crate::search_tool_routes::body::decode_body;
use crate::user_invitation::InvitationRecord;

#[derive(Deserialize)]
struct RemoveInvitation {
    invitation_id: String,
}

pub(crate) async fn delete_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> RouteResult {
    let Some(Extension(context)) = caller else {
        return Err(InvitationRouteError::Unauthenticated);
    };
    let request: RemoveInvitation = decode_body(&body).map_err(InvitationRouteError::Body)?;
    let record = remove_invitation(&state, &context.principal, &request.invitation_id).await?;
    Ok(Json(record).into_response())
}

async fn remove_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    invitation_id: &str,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state.database()?;
    let proxy_admin = principal.roles.contains(&Role::PlatformAdmin);
    let other_admin = database.scope.has_admin_privileges(principal).await;
    if !proxy_admin && !other_admin {
        return Err(InvitationRouteError::NotAllowed {
            role: role_text(principal),
        });
    }
    if other_admin && !proxy_admin {
        let creator = creator_of(database, invitation_id).await?;
        if Some(creator.as_str()) != principal.user.as_ref().map(|user| user.id.as_str()) {
            return Err(InvitationRouteError::OrgAdminDelete);
        }
    }
    database
        .invitations
        .delete(invitation_id)
        .await
        .map_err(InvitationRouteError::Store)?
        .ok_or(InvitationRouteError::InvitationMissing)
}

// an org admin may only delete the invitations it created
async fn creator_of(
    database: &InvitationDatabase,
    invitation_id: &str,
) -> Result<String, InvitationRouteError> {
    let found = database
        .invitations
        .info(invitation_id)
        .await
        .map_err(InvitationRouteError::Store)?
        .ok_or(InvitationRouteError::InvitationMissing)?;
    Ok(found.created_by)
}
