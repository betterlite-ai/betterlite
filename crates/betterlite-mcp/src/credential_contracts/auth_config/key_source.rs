use secrecy::SecretString;

use crate::credential_contracts::header_carrier::HeaderCarrier;

#[derive(Debug, Clone)]
pub struct SharedKey {
    pub value: SecretString,
}

#[derive(Debug, Clone)]
pub enum ApiKeySource {
    Shared(SharedKey),
    Byok,
}

#[derive(Debug, Clone)]
pub struct ApiKeyConfig {
    pub header: HeaderCarrier,
    pub key_source: ApiKeySource,
}
