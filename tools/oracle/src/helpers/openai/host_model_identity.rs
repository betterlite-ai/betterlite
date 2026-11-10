use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::host_model_identity::error::HostModelIdentityError;
use betterlite_openai::host_model_identity::identity::{
    SUPPORTED_OPENAI_PARAMS, clarifai_base_model, clarifai_request_model, datarobot_api_base,
};
use betterlite_openai::host_model_identity::{
    HostReply, HostReplyMode, PostCall, PostCallLogger, ReplySource, process_host_response,
};
use betterlite_openai::openai_base_config::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Identity,
    Reply,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Host {
    Clarifai,
    Compactifai,
    Datarobot,
}

#[derive(Deserialize)]
struct Step {
    kind: Kind,
    host: Host,
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
    fn post_call(&mut self, call: PostCall<'_>) {
        let api_key = call.api_key.map_or(Value::Null, |key| {
            Value::String(key.expose_secret().to_owned())
        });
        self.0.push(json!({
            "name": "post_call",
            "input": call.input,
            "api_key": api_key,
            "original_response": call.original_response,
            "complete_input_dict": call.complete_input_dict,
        }));
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        input
            .steps
            .iter()
            .map(step_value)
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()
            .map(Value::Array)
    }
}

fn step_value(step: &Step) -> Result<Value, Box<crate::helpers::HelperError>> {
    match step.kind {
        Kind::Identity => identity(step),
        Kind::Reply => Ok(reply(step)),
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

fn identity(step: &Step) -> Result<Value, Box<crate::helpers::HelperError>> {
    let settings = ConfigLoader {
        lookup: step.env.clone(),
    }
    .load("")
    .map(|config| config.sections.host_identity_settings)
    .map_err(|error| invalid(error.to_string()))?;
    let explicit = step.api_key.as_deref().map(SecretString::from);
    let base = step.api_base.as_deref();
    Ok(match step.host {
        Host::Clarifai => {
            let (api_base, api_key) = settings.resolve_clarifai(base, explicit.as_ref());
            json!({
                "host": "clarifai",
                "api_base": api_base,
                "api_key": api_key.expose_secret(),
                "supported": SUPPORTED_OPENAI_PARAMS,
                "models": step.models.iter().map(|model| clarifai_model(model.as_deref())).collect::<Vec<Value>>(),
            })
        }
        Host::Compactifai => {
            let (api_base, api_key) = settings.resolve_compactifai(base, explicit.as_ref());
            json!({"host": "compactifai", "api_base": api_base, "api_key": api_key.expose_secret()})
        }
        Host::Datarobot => {
            let (resolved, api_key) = settings.resolve_datarobot(base, explicit.as_ref());
            json!({
                "host": "datarobot",
                "api_base": datarobot_api_base(&resolved).map_err(|error| invalid(error.to_string()))?,
                "api_key": api_key.expose_secret(),
                // DataRobotConfig.get_complete_url is str(api_base), including the text form of None
                "complete_url": step.api_base.as_deref().unwrap_or("None"),
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
        _ => json!({"model": model, "ok": false, "base_model": null, "request_model": null}),
    }
}

fn reply(step: &Step) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    let messages = step
        .messages
        .clone()
        .unwrap_or_else(|| Json::List(Vec::new()));
    let mut recorder = Recorder(Vec::new());
    let outcome = dispatch_reply(step, &messages, key.as_ref(), &mut recorder);
    json!({
        "host": host_name(step.host),
        "events": recorder.0,
        "outcome": reply_outcome(outcome, &step.headers),
    })
}

fn dispatch_reply(
    step: &Step,
    messages: &Json,
    api_key: Option<&SecretString>,
    recorder: &mut Recorder,
) -> Result<ChatResponse, HostModelIdentityError> {
    let reply = HostReply {
        body: &step.body,
        status_code: step.status,
        headers: &step.headers,
        messages,
        api_key,
        request_data: &step.request_data,
        source: ReplySource {
            identifier: Uuid::nil,
            clock: || 1_700_000_000,
        },
    };
    let mode = match step.host {
        Host::Clarifai => HostReplyMode::Clarifai {
            request_model: &step.model,
        },
        Host::Compactifai => HostReplyMode::Compactifai {
            request_model: &step.model,
            json_mode: step.json_mode,
        },
        Host::Datarobot => return Err(HostModelIdentityError::Malformed { host: "datarobot" }),
    };
    process_host_response(mode, reply, recorder)
}

fn host_name(host: Host) -> &'static str {
    match host {
        Host::Clarifai => "clarifai",
        Host::Compactifai => "compactifai",
        Host::Datarobot => "datarobot",
    }
}

fn reply_outcome(
    outcome: Result<ChatResponse, HostModelIdentityError>,
    supplied: &BTreeMap<String, String>,
) -> Value {
    match outcome {
        Ok(response) => json!({"kind": "reply", "reply": reply_value(&response)}),
        Err(error) => error_outcome(&error, supplied),
    }
}

fn reply_value(response: &ChatResponse) -> Value {
    json!({
        "object": response.object,
        "model": response.model,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "choices": response.choices.iter().map(|choice| json!({
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "message": {
                "role": choice.message.role,
                "content": choice.message.content,
                "tool_calls": choice.message.tool_calls,
            },
        })).collect::<Vec<Value>>(),
    })
}

fn error_outcome(error: &HostModelIdentityError, supplied: &BTreeMap<String, String>) -> Value {
    match error {
        HostModelIdentityError::Api {
            status_code,
            headers,
            ..
        } => json!({
            "kind": "error",
            "status": status_code,
            "headers": supplied.keys().map(|name| (name.clone(), headers.get(name).cloned())).collect::<BTreeMap<String, Option<String>>>(),
        }),
        HostModelIdentityError::TooLarge
        | HostModelIdentityError::Json { .. }
        | HostModelIdentityError::Malformed { .. }
        | HostModelIdentityError::Reply { .. }
        | HostModelIdentityError::ModelName { .. }
        | HostModelIdentityError::ApiBase { .. } => {
            json!({"kind": "error", "status": null, "headers": null})
        }
    }
}
