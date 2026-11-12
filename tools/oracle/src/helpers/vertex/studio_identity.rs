use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_vertex::studio_identity::{
    AuthValue, NativeParams, get_api_base, get_api_key, native_headers, process_model_names,
};
use secrecy::{ExposeSecret, SecretString};
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
    auth_map: Option<BTreeMap<String, String>>,
    params_api_key: Option<String>,
    params_gemini_api_key: Option<String>,
    generic_key: Option<String>,
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let calls = input.calls.iter().map(resolve).collect::<Vec<_>>();
        Ok(json!({ "calls": calls }))
    }
}

fn settings(call: &Call) -> Settings {
    let environment_key = call
        .env
        .get("GOOGLE_API_KEY")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            call.env
                .get("GEMINI_API_KEY")
                .filter(|value| !value.is_empty())
        })
        .map(|value| SecretString::from(value.as_str()));
    Settings {
        environment_key,
        generic_api_key: call.generic_key.as_deref().map(SecretString::from),
        api_base: call
            .env
            .get("GEMINI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

fn explicit_key(call: &Call) -> Option<AuthValue> {
    if let Some(auth_map) = &call.auth_map {
        let headers = auth_map
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
            .collect();
        return Some(AuthValue::Headers(headers));
    }
    call.api_key
        .as_deref()
        .map(|key| AuthValue::Key(SecretString::from(key)))
}

fn native_params(call: &Call) -> NativeParams {
    NativeParams {
        api_key: call
            .params_api_key
            .as_deref()
            .map(|key| AuthValue::Key(SecretString::from(key))),
        gemini_api_key: call
            .params_gemini_api_key
            .as_deref()
            .map(|key| AuthValue::Key(SecretString::from(key))),
    }
}

fn resolve(call: &Call) -> Value {
    let settings = settings(call);
    let explicit = explicit_key(call);
    let api_key = get_api_key(
        call.api_key.as_deref().map(SecretString::from).as_ref(),
        &settings,
    );
    let headers = native_headers(
        explicit.as_ref(),
        &native_params(call),
        &settings,
        call.headers.clone(),
    );
    json!({
        "api_key": api_key.as_ref().map(ExposeSecret::expose_secret),
        "env_key": settings.environment_key.as_ref().map(ExposeSecret::expose_secret),
        "api_base": get_api_base(call.api_base.as_deref(), &settings),
        "headers": headers,
        "base_model": call.model.replace("gemini/", ""),
        "model_names": process_model_names(&call.models),
    })
}

mod tests;
