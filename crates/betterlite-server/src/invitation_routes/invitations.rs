mod create;
mod delete;
mod info;
mod update;

use axum::response::Response;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;

use super::InvitationRouteError;

pub(super) use self::create::new_invitation;
pub(super) use self::delete::delete_invitation;
pub(super) use self::info::invitation_info;
pub(super) use self::update::update_invitation;

pub(super) type RouteResult = Result<Response, InvitationRouteError>;

// LiteLLM's user_api_key_dict.user_role, as the role text its errors print
pub(super) fn role_text(principal: &Principal) -> &'static str {
    if principal.roles.contains(&Role::PlatformAdmin) {
        "proxy_admin"
    } else if principal.roles.contains(&Role::PlatformViewer) {
        "proxy_admin_view_only"
    } else if principal.roles.contains(&Role::OrgAdmin) {
        "org_admin"
    } else {
        "internal_user"
    }
}
