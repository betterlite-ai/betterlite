use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, TeamIdentity, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, TeamRole, map_role};
use betterlite_core::api::managed_resource_access::{
    ResourceRecord, build_list_page, build_owner_filter, can_access_resource,
    resolve_resource_owner_id,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    principal: PrincipalFields,
    #[serde(default)]
    records: Vec<ResourceRecord>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Deserialize)]
struct PrincipalFields {
    user_id: Option<String>,
    team_id: Option<String>,
    token: Option<String>,
    api_key: Option<String>,
    user_role: Option<String>,
}

impl From<&PrincipalFields> for Principal {
    fn from(fields: &PrincipalFields) -> Self {
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
            organization: None,
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
        let principal = Principal::from(&input.principal);
        let owner_filter =
            build_owner_filter(&principal).map_or(serde_json::Value::Null, |filter| match filter {
                betterlite_core::api::managed_resource_access::OwnerFilter::CreatedBy {
                    created_by,
                } => json!({"created_by": created_by}),
                betterlite_core::api::managed_resource_access::OwnerFilter::TeamId { team_id } => {
                    json!({"team_id": team_id})
                }
                betterlite_core::api::managed_resource_access::OwnerFilter::UserOrTeam {
                    user_id,
                    team_id,
                } => json!({"OR": [{"created_by": user_id}, {"team_id": team_id}]}),
                betterlite_core::api::managed_resource_access::OwnerFilter::Admin => json!({}),
            });
        let access = input
            .records
            .iter()
            .map(|record| {
                can_access_resource(
                    &principal,
                    record.created_by.as_deref(),
                    record.team_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "owner": resolve_resource_owner_id(&principal),
            "filter": owner_filter,
            "access": access,
            "page": build_list_page(input.records.clone(), input.has_more),
        }))
    }
}
