use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::ExposeSecret as _;

use super::error::PromptShieldError;
use crate::hook_initializers::secret_lookup::SecretLookup;

pub const ENV_PREFIX: &str = "os.environ/";

// LiteLLM's _resolved_secret_value: only an os.environ/ reference is resolved; anything else passes through.
pub fn resolved_secret_value(
    value: &GuardrailValue,
    lookup: &dyn SecretLookup,
) -> Result<GuardrailValue, PromptShieldError> {
    let GuardrailValue::Text(text) = value else {
        return Ok(value.clone());
    };
    Ok(GuardrailValue::Text(resolved_reference(text, lookup)?))
}

// LiteLLM's get_secret_strips the prefix before reading the variable, and an unset or blank value raises.
pub fn resolved_reference(
    text: &str,
    lookup: &dyn SecretLookup,
) -> Result<String, PromptShieldError> {
    let Some(name) = text.strip_prefix(ENV_PREFIX) else {
        return Ok(text.to_owned());
    };
    let Some(secret) = lookup.get_secret_str(name) else {
        return Err(PromptShieldError::UnsetSecret {
            reference: text.to_owned(),
        });
    };
    let resolved = secret.expose_secret().to_owned();
    if resolved.trim().is_empty() {
        return Err(PromptShieldError::UnsetSecret {
            reference: text.to_owned(),
        });
    }
    Ok(resolved)
}
