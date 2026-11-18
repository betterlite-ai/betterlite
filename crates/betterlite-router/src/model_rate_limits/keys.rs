use crate::deployment_identity::spec::DeploymentSpec;

use super::deployment::{model, model_id};

// _get_cache_keys: the tpm and rpm counters of one deployment in one UTC minute
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateKeys {
    pub tpm: String,
    pub rpm: String,
}

// ModelRateLimitingCheck._get_cache_keys builds the key with an f-string, so an absent id or model counts under the text None
#[must_use]
pub fn cache_keys(deployment: &DeploymentSpec, minute: &str) -> RateKeys {
    let id = model_id(deployment).unwrap_or("None");
    let name = model(deployment).unwrap_or("None");
    RateKeys {
        tpm: tpm_key(id, name, minute),
        rpm: format!("{id}:{name}:rpm:{minute}"),
    }
}

// ModelRateLimitingCheck.async_log_success_event builds its key from the standard logging payload's model id and resolved model
#[must_use]
pub fn tpm_key(model_id: &str, model: &str, minute: &str) -> String {
    format!("{model_id}:{model}:tpm:{minute}")
}
