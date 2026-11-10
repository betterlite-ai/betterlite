use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::host_model_identity::error::HostModelIdentityError;
use betterlite_openai::host_model_identity::{
    HostReplyMode, PostCall, PostCallLogger, SUPPORTED_OPENAI_PARAMS, clarifai_base_model,
    clarifai_request_model, datarobot_api_base, process_host_response,
};
use betterlite_openai::openai_base_config::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
struct Call {
    kind: String,
    host: String,
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<Option<String>>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    model: String,
    messages: Option<Json>,
    #[serde(default)]
    request_data: Object,
    #[serde(default)]
    json_mode: bool,
}

struct Recorder(Vec<Value>);

impl PostCallLogger for Recorder {
    fn post_call(&mut self, call: &PostCall<'_>) {
        let api_key = call.api_key.map_or(Value::Null, |key| {
            Value::String(key.expose_secret().to_owned())
        });
        self.0.push(json!([
            call.input,
            api_key,
            call.original_response,
            call.complete_input_dict
        ]));
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        input
            .calls
            .iter()
            .map(|call| {
                if call.kind == "reply" {
                    Ok(reply(call))
                } else {
                    identity(call)
                }
            })
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()
            .map(Value::Array)
    }
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn identity(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let settings = ConfigLoader {
        lookup: call.env.clone(),
    }
    .load("")
    .map(|config| config.sections.host_identity_settings)
    .map_err(|error| invalid(error.to_string()))?;
    let explicit = call.api_key.as_deref().map(SecretString::from);
    let base = call.api_base.as_deref();
    Ok(match call.host.as_str() {
        "clarifai" => {
            let (api_base, api_key) = settings.resolve_clarifai(base, explicit.as_ref());
            json!({
                "host": "clarifai",
                "api_base": api_base,
                "api_key": api_key.expose_secret(),
                "supported": SUPPORTED_OPENAI_PARAMS,
                "models": call.models.iter().map(|model| clarifai_model(model.as_deref())).collect::<Vec<Value>>(),
            })
        }
        "compactifai" => {
            let (api_base, api_key) = settings.resolve_compactifai(base, explicit.as_ref());
            json!({"host": "compactifai", "api_base": api_base, "api_key": api_key.expose_secret()})
        }
        _ => {
            let (resolved, api_key) = settings.resolve_datarobot(base, explicit.as_ref());
            json!({
                "host": "datarobot",
                "api_base": datarobot_api_base(&resolved).map_err(|error| invalid(error.to_string()))?,
                "api_key": api_key.expose_secret(),
                // DataRobotConfig.get_complete_url is str(api_base), including the text form of None
                "complete_url": call.api_base.as_deref().unwrap_or("None"),
            })
        }
    })
}

fn clarifai_model(model: Option<&str>) -> Value {
    match (clarifai_base_model(model), clarifai_request_model(model)) {
        (Ok(base), Ok(request_model)) => json!({
            "model": model,
            "ok": true,
            "base_model": base,
            "request_model": request_model,
        }),
        _ => json!({"model": model, "ok": false}),
    }
}

fn reply(call: &Call) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let messages = call
        .messages
        .clone()
        .unwrap_or_else(|| Json::List(Vec::new()));
    let mut recorder = Recorder(Vec::new());
    let reply = PostCall {
        input: &messages,
        api_key: key.as_ref(),
        original_response: &call.body,
        complete_input_dict: &call.request_data,
        status_code: call.status,
        headers: &call.headers,
    };
    let mode = match call.host.as_str() {
        "clarifai" => HostReplyMode::Clarifai {
            request_model: &call.model,
        },
        _ => HostReplyMode::Compactifai {
            request_model: &call.model,
            json_mode: call.json_mode,
        },
    };
    let outcome = process_host_response(mode, reply, &mut recorder);
    json!({"events": recorder.0, "outcome": outcome_value(outcome, &call.headers)})
}

fn outcome_value(
    outcome: Result<ChatResponse, HostModelIdentityError>,
    supplied: &BTreeMap<String, String>,
) -> Value {
    match outcome {
        Ok(response) => json!({
            "kind": "reply",
            "object": response.object,
            "model": response.model,
            "usage": [response.usage.prompt_tokens, response.usage.completion_tokens, response.usage.total_tokens],
            "choices": response.choices.iter().map(|choice| json!([
                choice.index,
                choice.finish_reason,
                choice.message.role,
                choice.message.content,
                choice.message.tool_calls,
            ])).collect::<Vec<Value>>(),
        }),
        Err(HostModelIdentityError::Api {
            status_code,
            headers,
            ..
        }) => json!({
            "kind": "error",
            "status": status_code,
            "headers": supplied.keys().map(|name| (name.clone(), headers.get(name).cloned())).collect::<BTreeMap<String, Option<String>>>(),
        }),
        Err(
            HostModelIdentityError::TooLarge
            | HostModelIdentityError::Json { .. }
            | HostModelIdentityError::Malformed { .. }
            | HostModelIdentityError::Reply { .. }
            | HostModelIdentityError::ModelName { .. }
            | HostModelIdentityError::ApiBase { .. },
        ) => {
            json!({"kind": "error", "status": null, "headers": null})
        }
    }
}
