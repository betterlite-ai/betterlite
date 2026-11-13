use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::error::WatsonxError;
use super::types::WatsonxParams;

// remove is false for a param another step still reads, and true for one this step consumes
pub(super) fn secret_from_params(
    params: &mut WatsonxParams,
    key: &'static str,
    remove: bool,
    environment_secret: Option<&SecretString>,
) -> Result<Option<SecretString>, WatsonxError> {
    let value = if remove {
        params.remove(key)
    } else {
        params.get(key).cloned()
    };
    let requested = match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => Some(SecretString::from(text)),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(WatsonxError::InvalidField { field: key });
        }
    };
    let fallback = environment_secret
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .cloned();
    Ok(requested.or(fallback))
}
