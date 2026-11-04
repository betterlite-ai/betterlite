use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, OrganizationIdentity, Principal, SourceKey, TeamIdentity,
    UserIdentity,
};
use betterlite_auth::auth_identity::roles::map_role;
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole};
use betterlite_core::api::managed_resource_store::{
    get_primary_owner_scope, get_resource_owner_scopes, is_proxy_admin,
    user_can_access_resource_owner,
};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(flatten)]
    query: Query,
    #[serde(default)]
    probes: Vec<Query>,
}

#[derive(Deserialize)]
struct Query {
    principal: Option<OwnerFields>,
    owner: Option<String>,
}

#[derive(Deserialize)]
struct OwnerFields {
    user_id: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
    api_key: Option<String>,
    token: Option<String>,
    user_role: Option<String>,
    #[serde(default, rename = "role_as_enum")]
    _role_as_enum: bool,
}

impl From<&OwnerFields> for Principal {
    fn from(fields: &OwnerFields) -> Self {
        Self {
            principal_type: PrincipalType::Human,
            subject: String::new(),
            issuer: None,
            audience: Vec::new(),
            user: fields.user_id.as_ref().map(|id| UserIdentity {
                id: id.clone(),
                external_id: None,
                user_name: None,
                email: None,
                display_name: None,
            }),
            organization: fields.org_id.as_ref().map(|id| OrganizationIdentity {
                id: id.clone(),
                name: None,
            }),
            teams: fields
                .team_id
                .iter()
                .map(|id| TeamIdentity {
                    id: id.clone(),
                    name: None,
                    role: TeamRole::Member,
                })
                .collect(),
            project: None,
            end_user: None,
            roles: map_role(fields.user_role.as_deref()).into_iter().collect(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef {
                key_id: fields.api_key.clone(),
                token_id: fields.token.clone(),
            },
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = evaluate(&input.query);
        let probes: Vec<_> = input.probes.iter().map(evaluate).collect();
        Ok(serde_json::json!({"result": value, "probes": probes}))
    }
}

#[derive(Serialize)]
struct Output {
    is_proxy_admin: bool,
    scopes: Vec<betterlite_core::api::managed_resource_store::OwnerScope>,
    primary: Option<betterlite_core::api::managed_resource_store::OwnerScope>,
    access: bool,
}

fn evaluate(query: &Query) -> Output {
    let identity = query.principal.as_ref().map(Principal::from);
    let principal = identity.as_ref();
    Output {
        is_proxy_admin: is_proxy_admin(principal),
        scopes: get_resource_owner_scopes(principal),
        primary: get_primary_owner_scope(principal),
        access: user_can_access_resource_owner(query.owner.as_deref(), principal),
    }
}
