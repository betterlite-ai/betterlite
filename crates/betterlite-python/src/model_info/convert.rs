use super::error::ModelInfoError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::time::Duration;
pub(crate) const DEFAULT_TIMEOUT_SECONDS: f64 = 5.0;
pub(crate) const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const fn default_timeout() -> f64 {
    DEFAULT_TIMEOUT_SECONDS
}
const fn default_max_attempts() -> u32 {
    DEFAULT_MAX_ATTEMPTS
}
// get_model_info's four arguments, as the Python layer sends them.
#[derive(Debug, Deserialize)]
pub(crate) struct ModelQuery {
    pub model: String,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct ProviderQuery {
    pub model: String,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
}
// The entries Python's model_cost changed since the last sync, plus the keys it deleted.
#[derive(Debug, Deserialize)]
pub(crate) struct RuntimeEntries {
    #[serde(default)]
    pub entries: IndexMap<String, Box<RawValue>>,
    #[serde(default)]
    pub removed: Vec<String>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct ReloadQuery {
    pub url: String,
    #[serde(default = "default_timeout")]
    pub timeout: f64,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}
impl ReloadQuery {
    pub(crate) fn timeout(&self) -> Result<Duration, ModelInfoError> {
        Duration::try_from_secs_f64(self.timeout).map_err(|source| ModelInfoError::Timeout {
            timeout: self.timeout,
            source,
        })
    }
}
// ast-grep-ignore: no-one-line-functions - one JSON decode shared by every Python request
pub(crate) fn parse<T>(text: &str) -> Result<T, ModelInfoError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).map_err(ModelInfoError::Request)
}
// The reload answers LiteLLM's ModelCostMapReloaded / ModelCostMapReloadUnavailable with.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReloadEnvelope<'answer> {
    Reloaded {
        model_cost_map: &'answer IndexMap<String, Box<RawValue>>,
        revision: Option<&'answer str>,
        etag: Option<&'answer str>,
    },
    Unavailable {
        reason: &'answer str,
    },
}
// ast-grep-ignore: no-one-line-functions - the one serializer for every reload answer
pub(crate) fn reload_result(envelope: &ReloadEnvelope<'_>) -> Result<String, ModelInfoError> {
    serde_json::to_string(envelope).map_err(ModelInfoError::Result)
}
mod tests;
