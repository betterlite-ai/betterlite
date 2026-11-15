use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;

use super::error::RegistryError;

pub fn params_changed(
    existing: Option<&Guardrail>,
    new: &Guardrail,
) -> Result<bool, RegistryError> {
    let Some(existing) = existing else {
        return Ok(true);
    };
    if existing.guardrail_name != new.guardrail_name {
        return Ok(true);
    }
    let existing_params = params_map(&existing.litellm_params)?;
    let new_params = params_map(&new.litellm_params)?;
    let mut changed = Vec::new();
    for key in existing_params.keys() {
        if value_of(&existing_params, key) != value_of(&new_params, key) {
            changed.push(key.clone());
        }
    }
    for key in new_params.keys() {
        if !existing_params.contains_key(key) {
            changed.push(key.clone());
        }
    }
    if changed.is_empty() {
        return Ok(false);
    }
    tracing::debug!(fields = ?changed, "guardrail litellm_params changed");
    Ok(true)
}

// a key missing on one side reads as null, as dict.get does in LiteLLM
fn value_of(params: &BTreeMap<String, GuardrailValue>, key: &str) -> GuardrailValue {
    params.get(key).cloned().unwrap_or(GuardrailValue::Null)
}

fn params_map(params: &LitellmParams) -> Result<BTreeMap<String, GuardrailValue>, RegistryError> {
    let bytes = serde_json::to_vec(params);
    let encoded = bytes.map_err(|source| RegistryError::Encode { source })?;
    let dump = serde_json::from_slice::<OpaqueParams>(&encoded);
    let decoded = dump.map_err(|source| RegistryError::Decode { source })?;
    Ok(decoded.0)
}
