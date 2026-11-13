use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultsProvider {
    Known(ProviderId),
    // LiteLLM's provider list has no anyscale member, so only api_base inference reaches that branch.
    Anyscale,
}

impl DefaultsProvider {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Known(provider) => <&'static str>::from(provider),
            Self::Anyscale => "anyscale",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderDefaultsRequest {
    pub provider: DefaultsProvider,
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub provider: DefaultsProvider,
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}
