use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_openai::groq_chat::credentials::compatible_provider_info;

use super::Call;

pub(super) fn credential(call: &Call) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let environment_key = call.environment_key.as_deref().map(SecretString::from);
    let (api_base, api_key) = compatible_provider_info(
        call.api_base.as_deref(),
        key.as_ref(),
        call.environment_base.as_deref(),
        environment_key.as_ref(),
    );
    json!({
        "api_base": api_base,
        "api_key": api_key.map(|secret| secret.expose_secret().to_owned()),
    })
}
