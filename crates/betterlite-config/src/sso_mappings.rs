use crate::loader::EnvironmentLookup;

mod tests;

pub const GENERIC_CLIENT_SECRET_ENV: &str = "GENERIC_CLIENT_SECRET";
pub const GENERIC_SCOPE_ENV: &str = "GENERIC_SCOPE";
pub const GENERIC_AUTHORIZATION_ENDPOINT_ENV: &str = "GENERIC_AUTHORIZATION_ENDPOINT";
pub const GENERIC_TOKEN_ENDPOINT_ENV: &str = "GENERIC_TOKEN_ENDPOINT";
pub const GENERIC_USERINFO_ENDPOINT_ENV: &str = "GENERIC_USERINFO_ENDPOINT";
pub const GENERIC_INCLUDE_CLIENT_ID_ENV: &str = "GENERIC_INCLUDE_CLIENT_ID";
pub const GENERIC_ROLE_MAPPINGS_ROLES_ENV: &str = "GENERIC_ROLE_MAPPINGS_ROLES";
pub const GENERIC_ROLE_MAPPINGS_GROUP_CLAIM_ENV: &str = "GENERIC_ROLE_MAPPINGS_GROUP_CLAIM";
pub const GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE_ENV: &str = "GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE";
pub const GENERIC_SSO_HEADERS_ENV: &str = "GENERIC_SSO_HEADERS";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SsoMappingSources {
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub userinfo_endpoint: Option<String>,
    pub include_client_id: bool,
    pub role_mappings_roles: Option<String>,
    pub role_mappings_group_claim: Option<String>,
    pub role_mappings_default_role: Option<String>,
    pub sso_headers: Option<String>,
}

#[must_use]
pub fn load_mapping_sources<L>(lookup: &L) -> SsoMappingSources
where
    L: EnvironmentLookup,
{
    // LiteLLM lowercases the flag and compares it with exactly "true"; padding leaves it off
    SsoMappingSources {
        client_secret: lookup.lookup(GENERIC_CLIENT_SECRET_ENV),
        scope: lookup.lookup(GENERIC_SCOPE_ENV),
        authorization_endpoint: lookup.lookup(GENERIC_AUTHORIZATION_ENDPOINT_ENV),
        token_endpoint: lookup.lookup(GENERIC_TOKEN_ENDPOINT_ENV),
        userinfo_endpoint: lookup.lookup(GENERIC_USERINFO_ENDPOINT_ENV),
        include_client_id: lookup
            .lookup(GENERIC_INCLUDE_CLIENT_ID_ENV)
            .is_some_and(|value| value.to_lowercase() == "true"),
        role_mappings_roles: lookup.lookup(GENERIC_ROLE_MAPPINGS_ROLES_ENV),
        role_mappings_group_claim: lookup.lookup(GENERIC_ROLE_MAPPINGS_GROUP_CLAIM_ENV),
        role_mappings_default_role: lookup.lookup(GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE_ENV),
        sso_headers: lookup.lookup(GENERIC_SSO_HEADERS_ENV),
    }
}
