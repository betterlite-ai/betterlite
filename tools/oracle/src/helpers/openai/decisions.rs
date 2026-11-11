use std::collections::BTreeMap;

use betterlite_models::decisions::openai;
use betterlite_models::decisions::systemone;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::openai::config::{Credential, OpenAiDecisionsConfig};
use betterlite_openai::decisions::openai::request::{ir_to_openai_request, openai_request_to_ir};
use betterlite_openai::decisions::openai::response::to_ir::openai_response_to_ir;
use betterlite_openai::decisions::openai::response::to_openai::ir_to_openai_response;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::decisions::transformation::requests::systemone_request_to_ir;
use betterlite_openai::decisions::transformation::responses::to_ir::systemone_response_to_ir;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod output;
mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    systemone_to_request: Vec<SystemOneToRequestCall>,
    #[serde(default)]
    request_roundtrip: Vec<RequestRoundtripCall>,
    #[serde(default)]
    response_to_ir: Vec<ResponseToIrCall>,
    #[serde(default)]
    ir_to_response: Vec<IrToResponseCall>,
    #[serde(default)]
    parse_response: Vec<ParseResponseCall>,
    #[serde(default)]
    settings: Vec<SettingsCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SystemOneToRequestCall {
    model: String,
    request: systemone::DecisionsRequestBody,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestRoundtripCall {
    model: String,
    // validated per call so a case can compare a rejected request body instead of failing to parse
    request: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseToIrCall {
    request: openai::DecisionRequestBody,
    response: openai::DecisionResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IrToResponseCall {
    requested_model: String,
    target: openai::DecisionRequestBody,
    source: Source,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
enum Source {
    OpenAi {
        request: openai::DecisionRequestBody,
        response: openai::DecisionResponse,
    },
    SystemOne {
        request: systemone::DecisionsRequestBody,
        response: systemone::DecisionsResponse,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseResponseCall {
    request: openai::DecisionRequestBody,
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsCall {
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    globals: GlobalsCall,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalsCall {
    api_base: Option<String>,
    api_key: Option<String>,
    openai_key: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({
            "systemone_to_request": input.systemone_to_request.iter().map(call_systemone_to_request).collect::<Vec<_>>(),
            "request_roundtrip": input.request_roundtrip.iter().map(call_request_roundtrip).collect::<Vec<_>>(),
            "response_to_ir": input.response_to_ir.iter().map(call_response_to_ir).collect::<Vec<_>>(),
            "ir_to_response": input.ir_to_response.iter().map(call_ir_to_response).collect::<Vec<_>>(),
            "parse_response": input.parse_response.iter().map(call_parse_response).collect::<Vec<_>>(),
            "settings": input.settings.iter().map(call_settings).collect::<Vec<_>>(),
        }))
    }
}

fn call_systemone_to_request(call: &SystemOneToRequestCall) -> Value {
    let request = systemone_request_to_ir(&call.request);
    match ir_to_openai_request(&call.model, &request) {
        Ok(wire) => wire_value(wire),
        Err(_unsupported) => json!({"ok": false}),
    }
}

fn call_request_roundtrip(call: &RequestRoundtripCall) -> Value {
    let Ok(body) = serde_json::from_value::<openai::DecisionRequestBody>(call.request.clone())
    else {
        return json!({"ok": false});
    };
    let request = openai_request_to_ir(&body);
    match ir_to_openai_request(&call.model, &request) {
        Ok(wire) => wire_value(wire),
        Err(_unsupported) => json!({"ok": false}),
    }
}

fn call_response_to_ir(call: &ResponseToIrCall) -> Value {
    let request = openai_request_to_ir(&call.request);
    let response = openai_response_to_ir(&call.response, &request);
    json!({"ok": true, "value": output::ir_response_json(&response)})
}

fn call_ir_to_response(call: &IrToResponseCall) -> Value {
    let target = openai_request_to_ir(&call.target);
    let response = match &call.source {
        Source::OpenAi { request, response } => {
            let source = openai_request_to_ir(request);
            openai_response_to_ir(response, &source)
        }
        Source::SystemOne { request, response } => {
            let source = systemone_request_to_ir(request);
            systemone_response_to_ir(response, &source)
        }
    };
    match ir_to_openai_response(&response, &target, &call.requested_model) {
        Ok(converted) => wire_value(converted),
        Err(_mismatch) => json!({"ok": false}),
    }
}

fn call_parse_response(call: &ParseResponseCall) -> Value {
    let request = openai_request_to_ir(&call.request);
    let config = OpenAiDecisionsConfig::default();
    let Ok(payload) = serde_json::to_vec(&call.payload) else {
        return json!({"ok": false});
    };
    match config.parse_response(&payload, &request) {
        Ok(response) => json!({"ok": true, "value": output::ir_response_json(&response)}),
        Err(_invalid) => json!({"ok": false}),
    }
}

fn call_settings(call: &SettingsCall) -> Value {
    let config = OpenAiDecisionsConfig {
        globals: LiteLlmGlobals {
            api_base: call.globals.api_base.clone(),
            api_key: call.globals.api_key.clone().map(SecretString::from),
            openai_key: call.globals.openai_key.clone().map(SecretString::from),
            organization: None,
        },
    };
    let lookup = |name: &str| call.env.get(name).cloned().unwrap_or_default();
    json!({
        "api_base": config.resolve(call.api_base.as_deref(), Credential::Base, &lookup),
        "api_key": config.resolve(call.api_key.as_deref(), Credential::Key, &lookup),
    })
}

fn wire_value<T>(wire: T) -> Value
where
    T: serde::Serialize,
{
    match serde_json::to_value(&wire) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(_encode) => json!({"ok": false}),
    }
}
