use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny};

use super::records::{OverridePath, Selector, UnregisteredClientBehavior, UserRole};
use crate::sections::auth_user_lookup::RbacRole;

pub(super) type Extras = BTreeMap<String, IgnoredAny>;
pub(super) type RawList<T> = Supplied<Vec<Supplied<T>>>;

// a key keeps its YAML shape until construction, so LiteLLM's check order decides which error wins
#[derive(Debug)]
pub(super) enum Supplied<T> {
    Absent,
    Null,
    Value(T),
    Invalid,
}

impl<T> Default for Supplied<T> {
    fn default() -> Self {
        Self::Absent
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Shape<T> {
    Value(T),
    Other(IgnoredAny),
}

impl<'de, T> Deserialize<'de> for Supplied<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<Shape<T>>::deserialize(deserializer)? {
            None => Self::Null,
            Some(Shape::Value(value)) => Self::Value(value),
            Some(Shape::Other(_)) => Self::Invalid,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawFlag {
    Bool(bool),
    Text(String),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawNumber {
    Number(f64),
    Text(String),
}

// the keyword arguments LiteLLM_JWTAuth receives, grouped as Settings groups them
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawJwtAuth {
    #[serde(flatten)]
    pub(super) admin: RawAdmin,
    #[serde(flatten)]
    pub(super) teams: RawTeams,
    #[serde(flatten)]
    pub(super) claims: RawClaims,
    #[serde(flatten)]
    pub(super) keys: RawKeys,
    #[serde(flatten)]
    pub(super) rbac: RawRbac,
    #[serde(flatten)]
    pub(super) scopes: RawScopes,
    pub(super) custom_validate: Supplied<String>,
    #[serde(flatten)]
    pub(super) sync: RawSync,
    #[serde(flatten)]
    pub(super) userinfo: RawUserinfo,
    #[serde(flatten)]
    pub(super) virtual_keys: RawVirtualKeys,
    #[serde(flatten)]
    pub(super) routing: RawRouting,
    pub(super) jwt_client_id_field: Supplied<String>,
    #[serde(flatten)]
    pub(super) unknown: Extras,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawAdmin {
    pub(super) admin_jwt_scope: Supplied<String>,
    pub(super) admin_allowed_routes: RawList<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawTeams {
    pub(super) team_id_jwt_field: Supplied<String>,
    pub(super) team_id_upsert: Supplied<RawFlag>,
    pub(super) team_ids_jwt_field: Supplied<String>,
    pub(super) upsert_sso_user_to_team: Supplied<RawFlag>,
    pub(super) team_allowed_routes: RawList<String>,
    pub(super) team_id_default: Supplied<String>,
    pub(super) team_alias_jwt_field: Supplied<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawClaims {
    pub(super) org_id_jwt_field: Supplied<String>,
    pub(super) org_alias_jwt_field: Supplied<String>,
    pub(super) user_id_jwt_field: Supplied<String>,
    pub(super) user_email_jwt_field: Supplied<String>,
    pub(super) user_allowed_email_domain: Supplied<String>,
    pub(super) user_roles_jwt_field: Supplied<String>,
    pub(super) user_allowed_roles: RawList<String>,
    pub(super) user_id_upsert: Supplied<RawFlag>,
    pub(super) end_user_id_jwt_field: Supplied<String>,
    pub(super) agent_id_jwt_field: Supplied<String>,
    pub(super) mcp_client_id_jwt_field: Supplied<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawKeys {
    pub(super) public_key_ttl: Supplied<RawNumber>,
    pub(super) public_key_stale_ttl: Supplied<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRbac {
    pub(super) public_allowed_routes: RawList<String>,
    pub(super) enforce_rbac: Supplied<RawFlag>,
    pub(super) roles_jwt_field: Supplied<String>,
    pub(super) role_mappings: RawList<RawRoleMapping>,
    pub(super) object_id_jwt_field: Supplied<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawScopes {
    pub(super) scope_mappings: RawList<RawScopeMapping>,
    pub(super) enforce_scope_based_access: Supplied<RawFlag>,
    pub(super) enforce_team_based_model_access: Supplied<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawSync {
    pub(super) jwt_litellm_role_map: RawList<RawRoleMap>,
    pub(super) sync_user_role_and_teams: Supplied<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawUserinfo {
    pub(super) oidc_userinfo_endpoint: Supplied<String>,
    pub(super) oidc_userinfo_enabled: Supplied<RawFlag>,
    pub(super) oidc_userinfo_cache_ttl: Supplied<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawVirtualKeys {
    pub(super) virtual_key_claim_field: Supplied<String>,
    pub(super) virtual_key_mapping_cache_ttl: Supplied<RawNumber>,
    pub(super) unregistered_jwt_client_behavior: Supplied<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRouting {
    pub(super) routing_overrides: RawList<RawRoutingOverride>,
    pub(super) team_claim_fallback: Supplied<RawFlag>,
    pub(super) fallback_to_db_teams: Supplied<RawFlag>,
    pub(super) issuers: RawList<RawIssuer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRoleMapping {
    pub(super) role: Supplied<String>,
    pub(super) internal_role: Supplied<RbacRole>,
    #[serde(flatten)]
    pub(super) extra: Extras,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRoleMap {
    pub(super) jwt_role: Supplied<String>,
    pub(super) litellm_role: Supplied<UserRole>,
    #[serde(flatten)]
    pub(super) extra: Extras,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawScopeMapping {
    pub(super) models: RawList<String>,
    pub(super) routes: RawList<String>,
    pub(super) scope: Supplied<String>,
    #[serde(flatten)]
    pub(super) extra: Extras,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRoutingOverride {
    pub(super) iss: Supplied<Selector>,
    pub(super) client_id: Supplied<Selector>,
    pub(super) scope: Supplied<Selector>,
    pub(super) aud: Supplied<Selector>,
    pub(super) path: Supplied<OverridePath>,
    #[serde(flatten)]
    pub(super) extra: Extras,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawIssuer {
    pub(super) issuer: Supplied<String>,
    pub(super) jwks_url: Supplied<String>,
    pub(super) audience: Supplied<Selector>,
    pub(super) disable_audience_validation: Supplied<bool>,
    pub(super) user_id_jwt_field: Supplied<String>,
    pub(super) user_email_jwt_field: Supplied<String>,
    pub(super) team_id_jwt_field: Supplied<String>,
    pub(super) team_ids_jwt_field: Supplied<String>,
    pub(super) org_id_jwt_field: Supplied<String>,
    pub(super) end_user_id_jwt_field: Supplied<String>,
    pub(super) virtual_key_claim_field: Supplied<String>,
    pub(super) unregistered_jwt_client_behavior: Supplied<UnregisteredClientBehavior>,
    #[serde(flatten)]
    pub(super) extra: Extras,
}
