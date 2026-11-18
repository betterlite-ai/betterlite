use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

mod tests;

pub const CLIENT_SECRET_ENV: &str = "GENERIC_CLIENT_SECRET";
pub const SCOPE_ENV: &str = "GENERIC_SCOPE";
pub const AUTHORIZATION_ENDPOINT_ENV: &str = "GENERIC_AUTHORIZATION_ENDPOINT";
pub const TOKEN_ENDPOINT_ENV: &str = "GENERIC_TOKEN_ENDPOINT";
pub const USERINFO_ENDPOINT_ENV: &str = "GENERIC_USERINFO_ENDPOINT";
pub const INCLUDE_CLIENT_ID_ENV: &str = "GENERIC_INCLUDE_CLIENT_ID";
pub const ROLE_MAPPINGS_ROLES_ENV: &str = "GENERIC_ROLE_MAPPINGS_ROLES";
pub const ROLE_MAPPINGS_GROUP_CLAIM_ENV: &str = "GENERIC_ROLE_MAPPINGS_GROUP_CLAIM";
pub const ROLE_MAPPINGS_DEFAULT_ROLE_ENV: &str = "GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE";
pub const SSO_HEADERS_ENV: &str = "GENERIC_SSO_HEADERS";

#[derive(Debug, Clone, Default)]
pub struct SsoMappingSources {
    pub client_secret: Option<SecretString>,
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
    SsoMappingSources {
        client_secret: lookup.lookup(CLIENT_SECRET_ENV).map(SecretString::from),
        scope: lookup.lookup(SCOPE_ENV),
        authorization_endpoint: lookup.lookup(AUTHORIZATION_ENDPOINT_ENV),
        token_endpoint: lookup.lookup(TOKEN_ENDPOINT_ENV),
        userinfo_endpoint: lookup.lookup(USERINFO_ENDPOINT_ENV),
        // LiteLLM lowercases the flag and compares it with exactly "true"; padding leaves it off
        include_client_id: lookup
            .lookup(INCLUDE_CLIENT_ID_ENV)
            .is_some_and(|value| value.to_lowercase() == "true"),
        role_mappings_roles: lookup.lookup(ROLE_MAPPINGS_ROLES_ENV),
        role_mappings_group_claim: lookup.lookup(ROLE_MAPPINGS_GROUP_CLAIM_ENV),
        role_mappings_default_role: lookup.lookup(ROLE_MAPPINGS_DEFAULT_ROLE_ENV),
        sso_headers: lookup.lookup(SSO_HEADERS_ENV),
    }
}
