use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::{LogInput, NEW_ID, system_clock};
use betterlite_openai::host_model_identity::error::HostModelIdentityError;
use betterlite_openai::host_model_identity::{
    HostReply, HostReplyMode, ReplyDefaults, SUPPORTED_OPENAI_PARAMS, clarifai_base_model,
    clarifai_request_model, datarobot_api_base, process_host_response,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Identity {
        host: IdentityHost,
        api_base: Option<String>,
        api_key: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        models: Vec<Option<String>>,
    },
    Reply(ReplyCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    host: ReplyHost,
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
    request_data: BTreeMap<String, Json>,
    #[serde(default)]
    json_mode: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum IdentityHost {
    Clarifai,
    Compactifai,
    Datarobot,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyHost {
    Clarifai,
    Compactifai,
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        input
            .calls
            .iter()
            .map(|call| match call {
                Call::Identity {
                    host,
                    api_base,
                    api_key,
                    env,
                    models,
                } => identity(host, api_base.as_deref(), api_key.as_deref(), env, models),
                Call::Reply(fields) => reply(fields),
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

fn identity(
    host: &IdentityHost,
    api_base: Option<&str>,
    api_key: Option<&str>,
    env: &BTreeMap<String, String>,
    models: &[Option<String>],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let settings = ConfigLoader {
        lookup: env.clone(),
    }
    .load("")
    .map(|config| config.sections.host_identity_settings)
    .map_err(|error| invalid(error.to_string()))?;
    let explicit = api_key.map(SecretString::from);
    Ok(match host {
        IdentityHost::Clarifai => {
            let (resolved, key) = settings.resolve_clarifai(api_base, explicit.as_ref());
            json!({
                "host": "clarifai",
                "api_base": resolved,
                "api_key": key.expose_secret(),
                "supported": SUPPORTED_OPENAI_PARAMS,
                "models": models.iter().map(|model| clarifai_model(model.as_deref())).collect::<Vec<Value>>(),
            })
        }
        IdentityHost::Compactifai => {
            let (resolved, key) = settings.resolve_compactifai(api_base, explicit.as_ref());
            json!({"host": "compactifai", "api_base": resolved, "api_key": key.expose_secret()})
        }
        IdentityHost::Datarobot => {
            let (resolved, key) = settings.resolve_datarobot(api_base, explicit.as_ref());
            json!({
                "host": "datarobot",
                "api_base": datarobot_api_base(&resolved).map_err(|error| invalid(error.to_string()))?,
                "api_key": key.expose_secret(),
                // DataRobotConfig.get_complete_url is str(api_base), including the text form of None
                "complete_url": api_base.unwrap_or("None"),
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

fn with_log<T, A>(action: A) -> Result<(T, LoggingState), Box<crate::helpers::HelperError>>
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| invalid(error.to_string()))?;
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(|error| invalid(error.to_string()))?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| invalid(error.to_string()))?,
    };
    let result = action(&mut state, &env);
    Ok((result, state))
}

fn reply(call: &ReplyCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mode = match call.host {
        ReplyHost::Clarifai => HostReplyMode::Clarifai {
            request_model: &call.model,
        },
        ReplyHost::Compactifai => HostReplyMode::Compactifai {
            request_model: &call.model,
            json_mode: call.json_mode,
        },
    };
    let reply = HostReply {
        mode,
        defaults: ReplyDefaults {
            identifier: NEW_ID,
            clock: system_clock,
        },
        status_code: call.status,
        headers: &call.headers,
    };
    let input = LogInput {
        messages: call
            .messages
            .clone()
            .unwrap_or_else(|| Json::List(Vec::new())),
        data: RawBody::Object(call.request_data.clone().into_iter().collect()),
    };
    let (outcome, state) =
        with_log(|state, env| process_host_response(&call.body, &reply, input, state, env))?;
    let details = &state.model_call_details;
    let complete_input_dict = serde_json::to_value(&details["additional_args"])
        .map_err(|error| invalid(error.to_string()))?
        .get("complete_input_dict")
        .cloned();
    Ok(json!({
        "events": [[details["input"], details["original_response"], complete_input_dict]],
        "outcome": outcome_value(outcome, &call.headers),
    }))
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
        Err(HostModelIdentityError::ClarifaiJson {
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
            | HostModelIdentityError::Clock { .. }
            | HostModelIdentityError::ModelName { .. }
            | HostModelIdentityError::ApiBase { .. },
        ) => {
            json!({"kind": "error", "status": null, "headers": null})
        }
    }
}
