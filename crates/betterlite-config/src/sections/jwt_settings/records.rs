use serde::{Deserialize, Serialize};
use serde_with::{As, DefaultOnNull};

use crate::sections::auth_user_lookup::RbacRole;

// RoleMapping and JWTLiteLLMRoleMap are plain BaseModels, so their extra keys are dropped
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMapping {
    pub role: String,
    pub internal_role: RbacRole,
}

// litellm/proxy/_types.py's LitellmUserRoles; betterlite-auth's UserRole sits above this crate
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    ProxyAdmin,
    ProxyAdminViewer,
    OrgAdmin,
    InternalUser,
    InternalUserViewer,
    Team,
    Customer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMap {
    pub jwt_role: String,
    pub litellm_role: UserRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeMapping {
    pub models: Option<Vec<String>>,
    pub routes: Option<Vec<String>>,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Selector {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverridePath {
    #[default]
    #[serde(rename = "oauth2")]
    Oauth2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingOverride {
    pub iss: Selector,
    pub client_id: Option<Selector>,
    pub scope: Option<Selector>,
    pub aud: Option<Selector>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub path: OverridePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnregisteredClientBehavior {
    FallbackTeamMapping,
    Reject,
    AutoRegister,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerConfig {
    pub issuer: String,
    pub jwks_url: Option<String>,
    pub audience: Option<Selector>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub disable_audience_validation: bool,
    pub user_id_jwt_field: Option<String>,
    pub user_email_jwt_field: Option<String>,
    pub team_id_jwt_field: Option<String>,
    pub team_ids_jwt_field: Option<String>,
    pub org_id_jwt_field: Option<String>,
    pub end_user_id_jwt_field: Option<String>,
    pub virtual_key_claim_field: Option<String>,
    #[serde(rename = "unregistered_jwt_client_behavior")]
    pub unregistered_client: Option<UnregisteredClientBehavior>,
}
