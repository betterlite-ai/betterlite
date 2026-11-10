use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::host_identity_settings::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::{ChatChoice, ChatResponse};
use betterlite_openai::host_model_identity::clarifai::{
    self, Reply as ClarifaiReply, SUPPORTED_OPENAI_PARAMS,
};
use betterlite_openai::host_model_identity::compactifai::{self, Reply as CompactifaiReply};
use betterlite_openai::host_model_identity::datarobot;
use betterlite_openai::host_model_identity::error::{HostModelIdentityError, get_error_class};
use betterlite_openai::host_model_identity::reply::{PostCall, PostCallLogger, ReplySource};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Identity(Identity),
    Reply(Reply),
    ErrorClass(ErrorClass),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Host {
    Clarifai,
    Compactifai,
    Datarobot,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyHost {
    Clarifai,
    Compactifai,
}

#[derive(Debug, Deserialize)]
struct Identity {
    host: Host,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
struct Reply {
    host: ReplyHost,
    body: String,
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    model: String,
    messages: Json,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    request_data: Map<String, Value>,
    #[serde(default)]
    json_mode: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorClass {
    host: ReplyHost,
    message: String,
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

struct Recorder(Vec<Value>);

impl PostCallLogger for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.0.push(json!({
            "name": "post_call",
            "input": call.input,
            "api_key": call.api_key.map(ExposeSecret::expose_secret),
            "original_response": call.original_response,
            "complete_input_dict": call.complete_input_dict,
        }));
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(match step {
                Step::Identity(identity) => identity_step(identity)?,
                Step::Reply(reply) => reply_step(reply)?,
                Step::ErrorClass(error) => error_class_step(error),
            });
        }
        Ok(Value::Array(results))
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

fn load_settings(
    env: &BTreeMap<String, String>,
) -> Result<Settings, Box<crate::helpers::HelperError>> {
    ConfigLoader {
        lookup: env.clone(),
    }
    .load("")
    .map(|config| config.sections.host_identity_settings)
    .map_err(|error| invalid(error.to_string()))
}

fn identity_step(step: &Identity) -> Result<Value, Box<crate::helpers::HelperError>> {
    let settings = load_settings(&step.env)?;
    let explicit = step.api_key.as_deref().map(SecretString::from);
    Ok(match step.host {
        Host::Clarifai => {
            let (api_base, api_key) =
                settings.resolve_clarifai(step.api_base.as_deref(), explicit.as_ref());
            json!({
                "host": "clarifai",
                "api_base": api_base,
                "api_key": api_key.expose_secret(),
                "supported": SUPPORTED_OPENAI_PARAMS,
                "models": step.models.iter().map(|model| clarifai_model(model.as_deref())).collect::<Vec<Value>>(),
            })
        }
        Host::Compactifai => {
            let (api_base, api_key) =
                settings.resolve_compactifai(step.api_base.as_deref(), explicit.as_ref());
            json!({
                "host": "compactifai",
                "api_base": api_base,
                "api_key": api_key.expose_secret(),
            })
        }
        Host::Datarobot => {
            let (resolved, api_key) =
                settings.resolve_datarobot(step.api_base.as_deref(), explicit.as_ref());
            let api_base = datarobot::transform_api_base(&resolved)
                .map_err(|error| invalid(error.to_string()))?;
            json!({
                "host": "datarobot",
                "api_base": api_base,
                "api_key": api_key.expose_secret(),
                // DataRobotConfig.get_complete_url is str(api_base), including the text form of None
                "complete_url": step.api_base.as_deref().unwrap_or("None"),
            })
        }
    })
}

fn clarifai_model(model: Option<&str>) -> Value {
    match (clarifai::base_model(model), clarifai::request_model(model)) {
        (Ok(base), Ok(request_model)) => json!({
            "model": model,
            "ok": true,
            "base_model": base,
            "request_model": request_model,
        }),
        _ => json!({
            "model": model,
            "ok": false,
            "base_model": null,
            "request_model": null,
        }),
    }
}

fn reply_step(step: &Reply) -> Result<Value, Box<crate::helpers::HelperError>> {
    let key = step.api_key.as_deref().map(SecretString::from);
    let mut recorder = Recorder(Vec::new());
    let outcome = process_reply(step, key.as_ref(), &mut recorder);
    Ok(json!({
        "host": reply_host_name(step.host),
        "events": recorder.0,
        "outcome": reply_outcome(outcome, &step.headers),
    }))
}

fn process_reply(
    step: &Reply,
    api_key: Option<&SecretString>,
    recorder: &mut Recorder,
) -> Result<ChatResponse, HostModelIdentityError> {
    let source = ReplySource {
        identifier: FIXED_ID,
        clock: FIXED_CLOCK,
    };
    match step.host {
        ReplyHost::Clarifai => clarifai::process_response(
            ClarifaiReply {
                body: &step.body,
                status_code: step.status,
                headers: &step.headers,
                request_model: &step.model,
                messages: &step.messages,
                api_key,
                request_data: &step.request_data,
                source,
            },
            recorder,
        ),
        ReplyHost::Compactifai => compactifai::process_response(
            CompactifaiReply {
                body: &step.body,
                request_model: &step.model,
                messages: &step.messages,
                api_key,
                request_data: &step.request_data,
                json_mode: step.json_mode,
                source,
            },
            recorder,
        ),
    }
}

fn reply_host_name(host: ReplyHost) -> &'static str {
    match host {
        ReplyHost::Clarifai => "clarifai",
        ReplyHost::Compactifai => "compactifai",
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
        "choices": response.choices.iter().map(choice_value).collect::<Vec<Value>>(),
    })
}

fn choice_value(choice: &ChatChoice) -> Value {
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "message": {
            "role": choice.message.role,
            "content": choice.message.content,
            "tool_calls": choice.message.tool_calls,
        },
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
            "headers": supplied
                .keys()
                .map(|name| (name.clone(), headers.get(name).cloned()))
                .collect::<BTreeMap<String, Option<String>>>(),
        }),
        HostModelIdentityError::TooLarge { .. }
        | HostModelIdentityError::Json { .. }
        | HostModelIdentityError::Malformed { .. }
        | HostModelIdentityError::Reply { .. }
        | HostModelIdentityError::ModelName { .. }
        | HostModelIdentityError::ApiBase { .. } => {
            json!({"kind": "error", "status": null, "headers": null})
        }
    }
}

fn error_class_step(step: &ErrorClass) -> Value {
    let error = get_error_class(&step.message, step.status, &step.headers);
    json!({
        "host": reply_host_name(step.host),
        "status": error.status_code,
        "message": error.message,
        "headers": step.headers,
    })
}

const FIXED_ID: fn() -> Uuid = Uuid::nil;
const FIXED_CLOCK: fn() -> i64 = || 1_700_000_000;
