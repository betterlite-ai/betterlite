mod tests;

use secrecy::SecretString;

use super::auth_config::AuthConfig;

#[derive(Debug, Clone)]
pub struct Subject {
    pub tenant_id: String,
    pub subject_id: String,
    pub inbound_token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub server_id: String,
    pub resource: String,
    pub config: AuthConfig,
}
