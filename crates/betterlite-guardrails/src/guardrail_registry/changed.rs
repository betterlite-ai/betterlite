use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;

use super::error::RegistryError;

pub fn params_changed(
    existing: Option<&Guardrail>,
    new: &Guardrail,
) -> Result<bool, RegistryError> {
    let Some(existing_guardrail) = existing else {
        return Ok(true);
    };
    if existing_guardrail.guardrail_name != new.guardrail_name {
        return Ok(true);
    }
    let existing_params = params_map(&existing_guardrail.litellm_params)?;
    let new_params = params_map(&new.litellm_params)?;
    // a key missing on one side reads as null, as dict.get does in LiteLLM
    let null = GuardrailValue::Null;
    let keys: BTreeSet<&String> = existing_params.keys().chain(new_params.keys()).collect();
    let mut changed = Vec::new();
    for key in keys {
        if existing_params.get(key).unwrap_or(&null) != new_params.get(key).unwrap_or(&null) {
            changed.push(key);
        }
    }
    if changed.is_empty() {
        return Ok(false);
    }
    tracing::debug!(fields = ?changed, "guardrail litellm_params changed");
    Ok(true)
}

fn params_map(params: &LitellmParams) -> Result<BTreeMap<String, GuardrailValue>, RegistryError> {
    let bytes = serde_json::to_vec(params);
    let encoded = bytes.map_err(|source| RegistryError::Encode { source })?;
    let dump = serde_json::from_slice::<OpaqueParams>(&encoded);
    let decoded = dump.map_err(|source| RegistryError::Decode { source })?;
    Ok(decoded.0)
}
