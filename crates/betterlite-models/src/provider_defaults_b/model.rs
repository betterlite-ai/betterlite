use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;

#[derive(Debug, Clone)]
pub struct ProviderDefaultsRequest {
    pub provider: ProviderId,
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}
