use secrecy::SecretString;

use crate::credential_contracts::header_carrier::HeaderCarrier;
use crate::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;

#[derive(Debug, Clone, Default)]
pub struct AuthorizationCodeConfig {
    pub header: HeaderCarrier,
    pub scopes: Vec<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ClientCredentialsConfig {
    pub header: HeaderCarrier,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub token_url: Option<String>,
    pub scopes: Vec<String>,
    pub audience: Option<String>,
    pub upstream_resource: Option<String>,
    pub token_endpoint_auth_method: Option<TokenEndpointAuthMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenExchangeProfile {
    Rfc8693,
    EntraObo,
}

pub const DEFAULT_SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

#[derive(Debug, Clone)]
pub struct TokenExchangeConfig {
    pub header: HeaderCarrier,
    pub profile: TokenExchangeProfile,
    pub subject_token_type: String,
    pub token_exchange_endpoint: Option<String>,
    pub audience: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub token_endpoint_auth_method: Option<TokenEndpointAuthMethod>,
    pub scopes: Vec<String>,
}

impl Default for TokenExchangeConfig {
    fn default() -> Self {
        Self {
            header: HeaderCarrier::default(),
            profile: TokenExchangeProfile::Rfc8693,
            subject_token_type: DEFAULT_SUBJECT_TOKEN_TYPE.to_owned(),
            token_exchange_endpoint: None,
            audience: None,
            client_id: None,
            client_secret: None,
            token_endpoint_auth_method: None,
            scopes: Vec::new(),
        }
    }
}
