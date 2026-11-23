use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;
use serde::Deserialize;

use super::state::InvitationRouteState;
use super::{Caller, InvitationRouteError};
use crate::api::agents::collection_routes::guards::is_proxy_admin;
use crate::auth::auth_resource_grants::denial_response;
use crate::search_tool_routes::body::decode_body;
use crate::tracing_routes::router::unauthenticated;
use crate::user_invitation::InvitationRecord;

const MASTER_KEY_SUBJECT: &str = "master_key";

#[derive(Deserialize)]
struct NewInvitation {
    user_id: String,
}

#[derive(Deserialize)]
struct AcceptInvitation {
    invitation_id: String,
    is_accepted: bool,
}

#[derive(Deserialize)]
struct RemoveInvitation {
    invitation_id: String,
}

#[derive(Deserialize)]
struct InfoQuery {
    invitation_id: Option<String>,
}

pub(super) async fn new_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let request: NewInvitation = match decode_body(&body) {
        Ok(decoded) => decoded,
        Err(error) => return InvitationRouteError::Body(error).into_response(),
    };
    match create_invitation(&state, &context.principal, &request.user_id).await {
        Ok(record) => Json(record).into_response(),
        Err(error) => denial_response(&error.wrapped()),
    }
}

pub(super) async fn invitation_info(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    query: Query<InfoQuery>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let principal = &context.principal;
    let Some(invitation_id) = query.invitation_id.as_deref() else {
        return InvitationRouteError::MissingInvitationId.into_response();
    };
    match read_invitation(&state, principal, invitation_id).await {
        Ok(record) => Json(record).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(super) async fn update_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let request: AcceptInvitation = match decode_body(&body) {
        Ok(decoded) => decoded,
        Err(error) => return InvitationRouteError::Body(error).into_response(),
    };
    match accept_invitation(&state, &context.principal, &request).await {
        Ok(record) => Json(record).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(super) async fn delete_invitation(
    State(state): State<Arc<InvitationRouteState>>,
    caller: Caller,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let request: RemoveInvitation = match decode_body(&body) {
        Ok(decoded) => decoded,
        Err(error) => return InvitationRouteError::Body(error).into_response(),
    };
    match remove_invitation(&state, &context.principal, &request.invitation_id).await {
        Ok(record) => Json(record).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn create_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    user_id: &str,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state
        .invitations
        .as_ref()
        .ok_or(InvitationRouteError::Database)?;
    let proxy_admin = is_proxy_admin(principal);
    let has_access = proxy_admin || database.scope.has_admin_privileges(principal).await;
    if !has_access {
        return Err(InvitationRouteError::NotAllowed {
            role: role_text(principal),
        });
    }
    if !proxy_admin && !database.scope.can_invite_user(principal, user_id).await {
        return Err(InvitationRouteError::InviteNotAllowed);
    }
    database
        .invitations
        .create(user_id, caller_user_id(principal))
        .await
        .map_err(InvitationRouteError::Store)
}

async fn read_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    invitation_id: &str,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state
        .invitations
        .as_ref()
        .ok_or(InvitationRouteError::Database)?;
    if !has_admin_view(principal) {
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

async fn accept_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    request: &AcceptInvitation,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state
        .invitations
        .as_ref()
        .ok_or(InvitationRouteError::Database)?;
    let Some(user_id) = caller_user_id(principal) else {
        return Err(InvitationRouteError::UserUnidentified);
    };
    database
        .invitations
        .update(&request.invitation_id, request.is_accepted, user_id)
        .await
        .map_err(InvitationRouteError::Store)?
        .ok_or(InvitationRouteError::InvitationMissing)
}

async fn remove_invitation(
    state: &InvitationRouteState,
    principal: &Principal,
    invitation_id: &str,
) -> Result<InvitationRecord, InvitationRouteError> {
    let database = state
        .invitations
        .as_ref()
        .ok_or(InvitationRouteError::Database)?;
    let proxy_admin = is_proxy_admin(principal);
    let other_admin = database.scope.has_admin_privileges(principal).await;
    if !proxy_admin && !other_admin {
        return Err(InvitationRouteError::NotAllowed {
            role: role_text(principal),
        });
    }
    if other_admin && !proxy_admin {
        let found = database
            .invitations
            .info(invitation_id)
            .await
            .map_err(InvitationRouteError::Store)?
            .ok_or(InvitationRouteError::InvitationMissing)?;
        if Some(found.created_by.as_str()) != caller_user_id(principal) {
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

fn caller_user_id(principal: &Principal) -> Option<&str> {
    principal.user.as_ref().map(|user| user.id.as_str())
}

// LiteLLM's user_api_key_dict.user_role, as the role text its errors print
fn role_text(principal: &Principal) -> &'static str {
    if is_proxy_admin(principal) {
        return "proxy_admin";
    }
    if principal.roles.contains(&Role::PlatformViewer) {
        return "proxy_admin_view_only";
    }
    if principal.roles.contains(&Role::OrgAdmin) {
        return "org_admin";
    }
    "internal_user"
}

// LiteLLM's _user_has_admin_view, plus the master key, which carries no key record to read a role from
fn has_admin_view(principal: &Principal) -> bool {
    principal.subject == MASTER_KEY_SUBJECT
        || principal
            .source_key
            .0
            .as_ref()
            .is_some_and(user_api_key_has_admin_view)
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer)
}
