use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::identity_rows::Payload;
use tracing::debug;

use super::KeyLookups;
use crate::auth_identity::roles::get_user_role;
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::identity_grants::denial::{PublicDenial, raise_public};
use crate::identity_grants::loaders::{MembershipLoader, TeamLoader, UserLoader};
use crate::identity_grants::records::{GrantOutcome, ResolvedGrants, UserQuery};
use crate::identity_grants::resolver::{GrantResolver, user_models};
use crate::identity_grants::team_grants::{ModelAliasMap, TeamGrants, team_grants};
use crate::resolved_key::auth::UserAPIKeyAuth;

// _refresh_session_token_grants: the live rows replace the minted grants, or the caller is refused.
pub async fn refresh_session_token_grants<Users, Teams, Members>(
    resolver: &GrantResolver<'_, Users, Teams, Members>,
    auth: &UserAPIKeyAuth,
) -> Result<UserAPIKeyAuth, PublicDenial>
where
    Users: UserLoader,
    Teams: TeamLoader,
    Members: MembershipLoader,
{
    let query = UserQuery {
        user_id: auth.view.token.user_id.clone(),
        user_email: None,
        sso_user_id: None,
        upsert: false,
    };
    match resolver
        .resolve(&query, auth.view.token.team_id.as_deref())
        .await
    {
        GrantOutcome::Resolved(grants) => {
            Ok(apply_grants(auth, &grants).unwrap_or_else(|| auth.clone()))
        }
        GrantOutcome::Degraded(error) => {
            debug!(
                error = %error,
                "Session token grants not refreshed, keeping minted grants"
            );
            Ok(auth.clone())
        }
        GrantOutcome::Denied(denial) => Err(raise_public(denial)),
    }
}

// A result without a typed live user keeps every minted field.
fn apply_grants(auth: &UserAPIKeyAuth, grants: &ResolvedGrants) -> Option<UserAPIKeyAuth> {
    let user = grants.identity.user.as_ref()?;
    let mut refreshed = auth.clone();
    if let Some(team) = grants.team.as_ref() {
        let grants = team_grants(
            &team.team,
            grants.identity.membership.as_ref(),
            Some(user.user_id.as_str()),
        );
        apply_team_grants(&mut refreshed, &grants);
        refreshed.view.token.models = Vec::new();
    } else {
        refreshed.view.token.models = user_models(user)
            .into_iter()
            .map(JsonData::String)
            .collect();
    }
    refreshed.extra.user_role = get_user_role(Some(user));
    Some(refreshed)
}

fn apply_team_grants(auth: &mut UserAPIKeyAuth, grants: &TeamGrants) {
    auth.view.team_alias = grants.team_alias.clone();
    auth.view.team_tpm_limit = grants.team_tpm_limit;
    auth.view.team_rpm_limit = grants.team_rpm_limit;
    auth.view.team_tpd_limit = grants.team_tpd_limit;
    auth.view.team_max_budget = grants.team_max_budget;
    auth.view.team_soft_budget = grants.team_soft_budget;
    auth.view.team_model_max_budget = grants.team_model_max_budget.clone().map(Payload);
    auth.view.team_spend = grants.team_spend;
    auth.view.team_models = grants
        .team_models
        .iter()
        .cloned()
        .map(JsonData::String)
        .collect();
    auth.view.team_blocked = grants.team_blocked;
    auth.view.team_metadata = grants.team_metadata.clone().map(Payload);
    auth.view.team_model_aliases = grants.team_model_aliases.clone().map(alias_payload);
    auth.view.team_object_permission_id = grants.team_object_permission_id.clone();
    auth.extra.team_object_permission = grants.team_object_permission.clone();
    auth.view.team_member = grants.team_member.clone();
    auth.view.team_member_spend = grants.team_member_spend;
    auth.view.team_member_tpm_limit = grants.team_member_tpm_limit;
    auth.view.team_member_rpm_limit = grants.team_member_rpm_limit;
}

fn alias_payload(aliases: ModelAliasMap) -> Payload {
    Payload(
        aliases
            .into_iter()
            .map(|(alias, model)| (alias, JsonData::String(model)))
            .collect(),
    )
}

// ast-grep-ignore: no-long-names - _resolve_object_permission_for_unresolvable_team: only a missing id or database answer None.
pub async fn resolve_object_permission_for_unresolvable_team(
    object_permission_id: Option<&str>,
    lookups: &KeyLookups,
) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
    let Some(object_permission_id) = object_permission_id else {
        return Ok(None);
    };
    let Some(permissions) = lookups.permissions.as_deref() else {
        return Ok(None);
    };
    permissions.permission(object_permission_id, false).await
}
