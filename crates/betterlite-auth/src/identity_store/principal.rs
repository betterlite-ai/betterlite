use betterlite_db::team_rows::member::MemberRole;

use crate::auth_identity::records::{
    CredentialRef, EndUserIdentity, NetworkContext, OrganizationIdentity, Principal,
    ProjectIdentity, SourceKey, TeamIdentity, UserIdentity,
};
use crate::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole, team_role};
use crate::auth_key_lookup::key_roles;
use crate::resolved_key::auth::UserAPIKeyAuth;

// The keyword arguments _principal_from_key takes beyond the key itself.
pub struct Projection<'input> {
    pub auth_method: AuthMethod,
    pub issuer: Option<&'input str>,
    pub subject_fallback: Option<&'input str>,
    pub scopes: &'input [String],
    pub credential_ref: Option<CredentialRef>,
    pub network: Option<NetworkContext>,
}

// The identity slice of an already-resolved key; pure, so the auth seam can project without a second read.
#[must_use]
pub fn principal_from_key(key: &UserAPIKeyAuth, projection: Projection<'_>) -> Principal {
    let view = &key.view;
    let user_id = view.token.user_id.clone();
    Principal {
        principal_type: if user_id.as_ref().is_some_and(|id| !id.is_empty()) {
            PrincipalType::Human
        } else {
            PrincipalType::ServiceAccount
        },
        subject: subject(user_id.as_deref(), key, projection.subject_fallback),
        issuer: projection.issuer.map(str::to_owned),
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id,
            external_id: None,
            user_name: None,
            email: key.extra.user_email.clone(),
            display_name: None,
        }),
        organization: view.token.org_id.clone().map(|id| OrganizationIdentity {
            id,
            name: view.organization_alias.clone(),
        }),
        teams: team_identities(key),
        project: view.token.project_id.clone().map(|id| ProjectIdentity {
            id,
            name: view.project_alias.clone(),
        }),
        end_user: view.end_user_id.clone().map(|id| EndUserIdentity { id }),
        roles: key_roles(key.extra.user_role),
        scopes: projection.scopes.to_vec(),
        auth_method: projection.auth_method,
        credential_ref: projection.credential_ref.unwrap_or_default(),
        network: projection.network.unwrap_or_default(),
        source_key: SourceKey(Some(key.clone())),
    }
}

// A nonnull team_id, including the empty one, creates the team; an absent member is a plain member.
fn team_identities(key: &UserAPIKeyAuth) -> Vec<TeamIdentity> {
    let Some(id) = key.view.token.team_id.as_ref() else {
        return Vec::new();
    };
    let role = key
        .view
        .team_member
        .as_ref()
        .map_or(TeamRole::Member, |member| {
            team_role(Some(member_role_text(&member.role)))
        });
    vec![TeamIdentity {
        id: id.clone(),
        name: key.view.team_alias.clone(),
        role,
    }]
}

// The first truthy of user_id, key_alias and the supplied fallback, then the empty subject.
fn subject(user_id: Option<&str>, key: &UserAPIKeyAuth, fallback: Option<&str>) -> String {
    let candidates = [user_id, key.view.token.key_alias.as_deref(), fallback];
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

const fn member_role_text(role: &MemberRole) -> &'static str {
    match role {
        MemberRole::Admin => "admin",
        MemberRole::User => "user",
    }
}
