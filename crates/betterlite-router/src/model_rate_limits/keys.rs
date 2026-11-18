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
        tpm: format!("{id}:{name}:tpm:{minute}"),
        rpm: format!("{id}:{name}:rpm:{minute}"),
    }
}
