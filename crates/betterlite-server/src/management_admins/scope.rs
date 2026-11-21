use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

// the two UserAPIKeyAuth fields the admin helpers read
pub struct AdminCaller<'caller> {
    pub user_id: Option<&'caller str>,
    pub proxy_admin: bool,
}

impl<'caller> AdminCaller<'caller> {
    #[must_use]
    pub fn from_principal(principal: &'caller Principal) -> Self {
        Self {
            user_id: principal.user.as_ref().map(|user| user.id.as_str()),
            proxy_admin: principal.roles.contains(&Role::PlatformAdmin),
        }
    }
}

// access.py is_team_admin: an exact roster user id with role admin, never an email or a None id
#[must_use]
pub fn is_team_admin(caller: &AdminCaller<'_>, team: &LiteLLMTeamTable) -> bool {
    let Some(caller_id) = caller.user_id else {
        return false;
    };
    team.fields.base.members_with_roles.iter().any(|member| {
        member.base.user_id.as_deref() == Some(caller_id) && member.role == MemberRole::Admin
    })
}
