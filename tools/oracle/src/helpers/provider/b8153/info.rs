use std::collections::BTreeMap;

use betterlite_provider::vllm::error::VllmSetupError;
use betterlite_provider::vllm::model_info::VllmModelInfo;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InfoInput {
    calls: Vec<InfoCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoCall {
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

fn setup_result<T>(result: Result<T, VllmSetupError>) -> Value
where
    T: Serialize,
{
    match result {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(_) => json!({"ok": false, "status": null}),
    }
}

fn model_info_value(call: &InfoCall) -> Value {
    let info = VllmModelInfo {
        environment: &call.env,
    };
    let mut headers = call.headers.clone();
    let key = call
        .api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    info.validate_environment(&mut headers, key.as_ref());
    let error = info.get_error_class("boom", 418, &BTreeMap::new());
    json!({
        "model": call.model,
        "api_base": setup_result(info.get_api_base(call.api_base.as_deref())),
        "models": setup_result(info.get_models(call.api_base.as_deref())),
        "headers": headers,
        "error": {"status": error.status()},
    })
}

pub(crate) fn model_info_values(input: &InfoInput) -> Value {
    let calls: Vec<Value> = input.calls.iter().map(model_info_value).collect();
    json!({"calls": calls})
}
