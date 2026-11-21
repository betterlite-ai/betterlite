use serde::{Deserialize, Serialize};

use super::records::{
    IssuerConfig, RoleMap, RoleMapping, RoutingOverride, ScopeMapping, UnregisteredClientBehavior,
};

// litellm/proxy/_types.py L5328 names this literal; no environment variable overrides it
pub const DEFAULT_JWKS_STALE_TTL: f64 = 3600.0;

const ADMIN_ROUTES: [&str; 4] = [
    "management_routes",
    "spend_tracking_routes",
    "global_spend_tracking_routes",
    "info_routes",
];

const TEAM_ROUTES: [&str; 5] = [
    "openai_routes",
    "info_routes",
    "mcp_routes",
    "/v1/messages",
    "/v1/messages/count_tokens",
];

// the scope and route lists, the only defaults among them that are not null or false
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteFields {
    pub admin_jwt_scope: String,
    pub admin_allowed_routes: Vec<String>,
    pub team_allowed_routes: Vec<String>,
    pub public_allowed_routes: Vec<String>,
}

impl Default for RouteFields {
    fn default() -> Self {
        Self {
            admin_jwt_scope: "litellm_proxy_admin".to_owned(),
            admin_allowed_routes: Vec::from(ADMIN_ROUTES.map(str::to_owned)),
            team_allowed_routes: Vec::from(TEAM_ROUTES.map(str::to_owned)),
            public_allowed_routes: vec!["public_routes".to_owned()],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamFields {
    pub team_id_jwt_field: Option<String>,
    pub team_id_upsert: bool,
    pub team_ids_jwt_field: Option<String>,
    pub upsert_sso_user_to_team: bool,
    pub team_id_default: Option<String>,
    pub team_alias_jwt_field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimFields {
    pub org_id_jwt_field: Option<String>,
    pub org_alias_jwt_field: Option<String>,
    pub user_id_jwt_field: Option<String>,
    pub user_email_jwt_field: Option<String>,
    pub user_allowed_email_domain: Option<String>,
    pub user_roles_jwt_field: Option<String>,
    pub user_allowed_roles: Option<Vec<String>>,
    pub user_id_upsert: bool,
    pub end_user_id_jwt_field: Option<String>,
    pub agent_id_jwt_field: Option<String>,
    pub mcp_client_id_jwt_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyFields {
    pub public_key_ttl: f64,
    pub public_key_stale_ttl: f64,
}

impl Default for KeyFields {
    fn default() -> Self {
        Self {
            public_key_ttl: 600.0,
            public_key_stale_ttl: DEFAULT_JWKS_STALE_TTL,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RbacFields {
    pub enforce_rbac: bool,
    pub roles_jwt_field: Option<String>,
    pub role_mappings: Option<Vec<RoleMapping>>,
    pub object_id_jwt_field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScopeFields {
    pub scope_mappings: Option<Vec<ScopeMapping>>,
    pub enforce_scope_based_access: bool,
    #[serde(rename = "enforce_team_based_model_access")]
    pub team_model_access: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncFields {
    pub jwt_litellm_role_map: Option<Vec<RoleMap>>,
    pub sync_user_role_and_teams: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserinfoFields {
    pub oidc_userinfo_endpoint: Option<String>,
    pub oidc_userinfo_enabled: bool,
    pub oidc_userinfo_cache_ttl: f64,
}

impl Default for UserinfoFields {
    fn default() -> Self {
        Self {
            oidc_userinfo_endpoint: None,
            oidc_userinfo_enabled: false,
            oidc_userinfo_cache_ttl: 300.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VirtualKeyFields {
    pub virtual_key_claim_field: Option<String>,
    pub virtual_key_mapping_cache_ttl: f64,
    #[serde(rename = "unregistered_jwt_client_behavior")]
    pub unregistered_client: UnregisteredClientBehavior,
}

impl Default for VirtualKeyFields {
    fn default() -> Self {
        Self {
            virtual_key_claim_field: None,
            virtual_key_mapping_cache_ttl: 300.0,
            unregistered_client: UnregisteredClientBehavior::FallbackTeamMapping,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingFields {
    pub routing_overrides: Option<Vec<RoutingOverride>>,
    pub team_claim_fallback: bool,
    pub fallback_to_db_teams: bool,
    pub issuers: Option<Vec<IssuerConfig>>,
}
