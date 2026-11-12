use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use error::SelectionError;

pub mod error;
mod tests;

// Guardrail name to the callback names it expands to, as LiteLLM's guardrail_name_config_map holds them.
pub type CallbackNames = BTreeMap<String, Vec<String>>;

// Only an explicit false in the team's guardrails metadata locks the guardrails.
#[must_use]
pub fn can_modify_guardrails(team_metadata: Option<&Object>) -> bool {
    let Some(metadata) = team_metadata else {
        return true;
    };
    let Some(GuardrailValue::Object(guardrails)) = metadata.get("guardrails") else {
        return true;
    };
    !matches!(
        guardrails.get("modify_guardrails"),
        Some(GuardrailValue::Boolean(false))
    )
}

// A request that names guardrails in metadata runs only the callbacks of the ones it did not set to false.
pub fn guardrail_applies(
    data: &Object,
    guardrail_name: &str,
    callback_names: &CallbackNames,
) -> Result<bool, SelectionError> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return Ok(true);
    };
    let Some(GuardrailValue::Object(requested)) = metadata.get("guardrails") else {
        return Ok(true);
    };
    let mut requested_callbacks: Vec<&String> = Vec::new();
    for (requested_name, should_run) in requested {
        if matches!(should_run, GuardrailValue::Boolean(false)) {
            continue;
        }
        let callbacks =
            callback_names
                .get(requested_name)
                .ok_or_else(|| SelectionError::UnknownGuardrail {
                    guardrail_name: requested_name.clone(),
                })?;
        requested_callbacks.extend(callbacks);
    }
    Ok(requested_callbacks
        .iter()
        .any(|callback| callback.as_str() == guardrail_name))
}
