mod tests;

use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use secrecy::SecretString;

pub const TOKEN_ENDPOINT_VARIABLE: &str = "GENERIC_TOKEN_ENDPOINT";
pub const CLIENT_ID_VARIABLE: &str = "GENERIC_CLIENT_ID";
pub const CLIENT_SECRET_VARIABLE: &str = "GENERIC_CLIENT_SECRET";
pub const INCLUDE_CLIENT_ID_VARIABLE: &str = "GENERIC_INCLUDE_CLIENT_ID";

#[derive(Debug)]
pub struct SsoClientConfig {
    pub token_endpoint: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub auth_method: TokenEndpointAuthMethod,
}

pub trait SsoClientSource: Send + Sync {
    fn client(&self) -> Option<SsoClientConfig>;
}

// Read on every refresh, the way LiteLLM reads os.environ on every call, so a settings change lands without a restart.
#[must_use]
pub fn sso_client_config(env: &BTreeMap<String, String>) -> Option<SsoClientConfig> {
    let token_endpoint = env.get(TOKEN_ENDPOINT_VARIABLE)?;
    let client_id = env.get(CLIENT_ID_VARIABLE)?;
    let client_secret = env.get(CLIENT_SECRET_VARIABLE)?;
    if token_endpoint.is_empty() || client_id.is_empty() || client_secret.is_empty() {
        return None;
    }
    let includes_client_id = env
        .get(INCLUDE_CLIENT_ID_VARIABLE)
        .is_some_and(|value| value.to_lowercase() == "true");
    let auth_method = if includes_client_id {
        TokenEndpointAuthMethod::ClientSecretPost
    } else {
        TokenEndpointAuthMethod::ClientSecretBasic
    };
    Some(SsoClientConfig {
        token_endpoint: token_endpoint.clone(),
        client_id: client_id.clone(),
        client_secret: SecretString::from(client_secret.clone()),
        auth_method,
    })
}
