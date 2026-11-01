use secrecy::SecretString;

use crate::credential_contracts::header_carrier::HeaderCarrier;

pub const ID_JAG_SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";
pub const PRIVATE_KEY_JWT_SIGNING_ALG: &str = "RS256";

#[derive(Debug, Clone)]
pub struct PrivateKeyJwtAuth {
    pub private_key: SecretString,
    pub key_id: Option<String>,
    pub signing_alg: String,
}

#[derive(Debug, Clone)]
pub struct ClientSecretAuth {
    pub client_secret: SecretString,
}

#[derive(Debug, Clone)]
pub enum ClientAuth {
    PrivateKeyJwt(PrivateKeyJwtAuth),
    ClientSecret(ClientSecretAuth),
}

#[derive(Debug, Clone)]
pub struct IdJagConfig {
    pub header: HeaderCarrier,
    pub org_token_endpoint: String,
    pub resource_token_endpoint: String,
    pub client_id: String,
    pub client_auth: ClientAuth,
    pub subject_token_type: String,
    pub audience: Option<String>,
    pub resource: Option<String>,
    pub scopes: Vec<String>,
}
