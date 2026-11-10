mod rest;
use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::spend_storage::{load_spend_storage_policy, runtime_max_prompt_length};
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::messages::storage_messages;
use betterlite_logging::spend_bodies::request::{RequestKwargs, storage_request_body};
use betterlite_logging::spend_bodies::response::{ResponseKwargs, storage_response};
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) function: String,
    pub(super) config_value: Option<Value>,
    pub(super) env_value: Option<String>,
    pub(super) initial_max_string_length: Option<String>,
    pub(super) runtime_max_string_lengths: Option<Vec<Option<String>>>,
    pub(super) store_bodies: Option<bool>,
    pub(super) max_string_length: Option<i64>,
    pub(super) turn_off_message_logging: Option<bool>,
    pub(super) litellm_params: Option<Value>,
    pub(super) standard_logging_object: Option<Value>,
    pub(super) realtime_tools: Option<Value>,
    pub(super) dynamic_params: Option<Value>,
    pub(super) response: Option<Value>,
    pub(super) realtime_tool_calls: Option<Value>,
    pub(super) messages_payload: Option<Value>,
    pub(super) snapshots: Option<Vec<SnapshotSpec>>,
    pub(super) vector_store: Option<Value>,
    pub(super) model_group: Option<String>,
    pub(super) raw_model: Option<String>,
    pub(super) sessions: Option<Vec<SessionCase>>,
    pub(super) durations: Option<Vec<DurationSpec>>,
    pub(super) timestamps: Option<Vec<String>>,
}
#[derive(serde::Deserialize)]
pub(crate) struct SnapshotSpec {
    pub(super) value: Value,
    pub(super) openai_sdk: bool,
}
#[derive(serde::Deserialize)]
pub(crate) struct DurationSpec {
    pub(super) start: String,
    pub(super) end: String,
}
#[derive(serde::Deserialize)]
pub(crate) struct SessionCase {
    pub(super) metadata: Option<Value>,
    pub(super) configured_omit: bool,
    pub(super) call_type: Option<String>,
    pub(super) request_id: Option<String>,
    pub(super) trace_id: Option<Value>,
    pub(super) litellm_trace_id: Option<Value>,
    pub(super) injected_uuid: String,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "storage_policy" => storage_policy(input),
            "body_storage" => body_storage(input),
            "classifier" => rest::classifier(input),
            "vector_and_placeholder" => rest::vector_and_placeholder(input),
            "session" => rest::session(input),
            _ => Err(invalid("unknown spend_bodies function")),
        }
    }
}
fn storage_policy(input: &Input) -> HelperResult {
    let mut environment = BTreeMap::new();
    if let Some(value) = &input.env_value {
        environment.insert("STORE_PROMPTS_IN_SPEND_LOGS".to_owned(), value.clone());
    }
    if let Some(value) = &input.initial_max_string_length {
        environment.insert("MAX_STRING_LENGTH_PROMPT_IN_DB".to_owned(), value.clone());
    }
    let config_value = required(input.config_value.as_ref(), "config_value")?;
    let document = json!({"general_settings": {"store_prompts_in_spend_logs": config_value}});
    let source = serde_saphyr::to_string(&document).map_err(|error| invalid(&error.to_string()))?;
    let settings = load_spend_storage_policy(&source, &environment)
        .map_err(|error| invalid(&error.to_string()))?;
    let initial_length = settings.max_string_length_prompt_in_db;
    let lengths = required(
        input.runtime_max_string_lengths.as_ref(),
        "runtime_max_string_lengths",
    )?
    .iter()
    .map(|snapshot| runtime_max_prompt_length(initial_length, snapshot.as_deref()))
    .collect::<Vec<_>>();
    Ok(json!({
        "store_bodies": settings.store_prompts_in_spend_logs,
        "max_string_lengths": lengths,
    }))
}
fn body_storage(input: &Input) -> HelperResult {
    let policy = policy_of(input)?;
    let params = decode(required(input.litellm_params.as_ref(), "litellm_params")?)?;
    let dynamic = decode_optional(input.dynamic_params.as_ref())?;
    let stored_response = decode(required(input.response.as_ref(), "response")?)?;
    let tool_calls = decode_optional(input.realtime_tool_calls.as_ref())?;
    let messages = decode_optional(input.messages_payload.as_ref())?;
    let payload = Json::Object(IndexMap::from([("response".to_owned(), stored_response)]));
    let response_kwargs = ResponseKwargs {
        realtime_tool_calls: tool_calls.as_ref(),
        litellm_params: Some(&params),
        dynamic_params: dynamic.as_ref(),
    };
    Ok(json!({
        "request": request_text(input, policy)?,
        "response": storage_response(Some(&payload), Some(&response_kwargs), policy),
        "messages": storage_messages(messages.as_ref(), policy.sanitize.store_bodies),
    }))
}
pub(super) fn request_text(
    input: &Input,
    policy: SpendStoragePolicy,
) -> Result<String, Box<HelperError>> {
    let params = decode(required(input.litellm_params.as_ref(), "litellm_params")?)?;
    let standard = decode_optional(input.standard_logging_object.as_ref())?;
    let tools = decode_optional(input.realtime_tools.as_ref())?;
    let dynamic = decode_optional(input.dynamic_params.as_ref())?;
    let kwargs = RequestKwargs {
        standard_logging_object: standard.as_ref(),
        realtime_tools: tools.as_ref(),
        dynamic_params: dynamic.as_ref(),
    };
    Ok(storage_request_body(&params, Some(&kwargs), policy))
}
pub(super) fn policy_of(input: &Input) -> Result<SpendStoragePolicy, Box<HelperError>> {
    Ok(SpendStoragePolicy {
        sanitize: SpendSanitizePolicy {
            store_bodies: required(input.store_bodies, "store_bodies")?,
            max_string_length: required(input.max_string_length, "max_string_length")?,
        },
        turn_off_message_logging: required(
            input.turn_off_message_logging,
            "turn_off_message_logging",
        )?,
    })
}
fn decode(value: &Value) -> Result<Json, Box<HelperError>> {
    let decoded =
        serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))?;
    Ok(decoded)
}
fn decode_optional(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    match value {
        Some(present) => Ok(Some(decode(present)?)),
        None => Ok(None),
    }
}
fn required<T>(value: Option<T>, name: &str) -> Result<T, Box<HelperError>> {
    let Some(present) = value else {
        return Err(invalid(name));
    };
    Ok(present)
}
fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
