use secrecy::SecretString;
use serde::Deserialize;

use crate::identity::provider_id::ProviderId;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteLlmParams {
    pub model: String,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<ProviderId>,
}
