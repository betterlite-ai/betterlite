use std::collections::BTreeMap;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_openai::xai_identity::{
    KeyOrder, get_api_base, get_api_key, get_base_model, validate_environment,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    api_key: Option<String>,
    xai_key: Option<String>,
    generic_key: Option<String>,
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = input.calls.iter().map(resolve).collect::<Vec<_>>();
        Ok(json!({ "calls": calls }))
    }
}

fn module_settings(call: &Call) -> Settings {
    Settings {
        xai_key: call.xai_key.as_deref().map(SecretString::from),
        generic_api_key: call.generic_key.as_deref().map(SecretString::from),
        environment_key: call
            .env
            .get("XAI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        api_base: call
            .env
            .get("XAI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

fn resolve(call: &Call) -> Value {
    let settings = module_settings(call);
    let explicit = call.api_key.as_deref().map(SecretString::from);
    let chat_key = get_api_key(explicit.as_ref(), KeyOrder::Chat, &settings);
    let legacy_key = get_api_key(
        explicit.as_ref(),
        KeyOrder::GenericBeforeEnvironment,
        &settings,
    );
    json!({
        "chat_key": chat_key.as_ref().map(ExposeSecret::expose_secret),
        "legacy_key": legacy_key.as_ref().map(ExposeSecret::expose_secret),
        "api_base": get_api_base(call.api_base.as_deref(), &settings),
        "headers": validate_environment(call.headers.clone(), explicit.as_ref()),
        "base_model": get_base_model(&call.model),
    })
}

mod tests;
