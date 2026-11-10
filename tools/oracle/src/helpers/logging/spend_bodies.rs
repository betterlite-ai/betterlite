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
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    StoragePolicy(StoragePolicyInput),
    BodyStorage(BodyStorageInput),
    Classifier(ClassifierInput),
    VectorAndPlaceholder(VectorInput),
    Session(SessionCall),
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoragePolicyInput {
    config_value: Json,
    env_value: Option<String>,
    initial_max_string_length: Option<String>,
    runtime_max_string_lengths: Vec<Option<String>>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyStorageInput {
    store_bodies: bool,
    max_string_length: i64,
    turn_off_message_logging: bool,
    litellm_params: Json,
    standard_logging_object: Option<Json>,
    realtime_tools: Option<Json>,
    dynamic_params: Option<Json>,
    response: Json,
    realtime_tool_calls: Option<Json>,
    messages_payload: Option<Json>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifierInput {
    store_bodies: bool,
    max_string_length: i64,
    turn_off_message_logging: bool,
    litellm_params: Json,
    standard_logging_object: Option<Json>,
    realtime_tools: Option<Json>,
    dynamic_params: Option<Json>,
    snapshots: Vec<SnapshotSpec>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorInput {
    store_bodies: bool,
    litellm_params: Json,
    model_group: String,
    raw_model: String,
    vector_store: Option<Json>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCall {
    sessions: Vec<SessionCase>,
    durations: Vec<DurationSpec>,
    timestamps: Vec<String>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotSpec {
    value: Json,
    openai_sdk: bool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DurationSpec {
    start: String,
    end: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCase {
    metadata: Option<Json>,
    configured_omit: bool,
    call_type: Option<String>,
    request_id: Option<String>,
    trace_id: Option<Json>,
    litellm_trace_id: Option<Json>,
    injected_uuid: String,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::StoragePolicy(call) => storage_policy(call),
            Input::BodyStorage(call) => Ok(body_storage(call)),
            Input::Classifier(call) => rest::classifier(call),
            Input::VectorAndPlaceholder(call) => rest::vector_and_placeholder(call),
            Input::Session(call) => {
                rest::session(&call.sessions, &call.durations, &call.timestamps)
            }
        }
    }
}
fn storage_policy(call: &StoragePolicyInput) -> HelperResult {
    let mut environment = BTreeMap::new();
    if let Some(value) = &call.env_value {
        environment.insert("STORE_PROMPTS_IN_SPEND_LOGS".to_owned(), value.clone());
    }
    if let Some(value) = &call.initial_max_string_length {
        environment.insert("MAX_STRING_LENGTH_PROMPT_IN_DB".to_owned(), value.clone());
    }
    let document = json!({"general_settings": {"store_prompts_in_spend_logs": call.config_value}});
    let source = serde_saphyr::to_string(&document).map_err(|error| invalid(&error.to_string()))?;
    let settings = load_spend_storage_policy(&source, &environment)
        .map_err(|error| invalid(&error.to_string()))?;
    let initial_length = settings.max_string_length_prompt_in_db;
    let lengths = call
        .runtime_max_string_lengths
        .iter()
        .map(|snapshot| runtime_max_prompt_length(initial_length, snapshot.as_deref()))
        .collect::<Vec<_>>();
    Ok(json!({
        "store_bodies": settings.store_prompts_in_spend_logs,
        "max_string_lengths": lengths,
    }))
}
fn body_storage(call: &BodyStorageInput) -> Value {
    let storage = SpendStoragePolicy {
        sanitize: SpendSanitizePolicy {
            store_bodies: call.store_bodies,
            max_string_length: call.max_string_length,
        },
        turn_off_message_logging: call.turn_off_message_logging,
    };
    let request = request_text(
        &call.litellm_params,
        call.standard_logging_object.as_ref(),
        call.realtime_tools.as_ref(),
        call.dynamic_params.as_ref(),
        storage,
    );
    let payload = Json::Object(IndexMap::from([(
        "response".to_owned(),
        call.response.clone(),
    )]));
    let response_kwargs = ResponseKwargs {
        realtime_tool_calls: call.realtime_tool_calls.as_ref(),
        litellm_params: Some(&call.litellm_params),
        dynamic_params: call.dynamic_params.as_ref(),
    };
    json!({
        "request": request,
        "response": storage_response(Some(&payload), Some(&response_kwargs), storage),
        "messages": storage_messages(call.messages_payload.as_ref(), storage.sanitize.store_bodies),
    })
}
pub(super) fn request_text(
    params: &Json,
    standard_logging_object: Option<&Json>,
    realtime_tools: Option<&Json>,
    dynamic_params: Option<&Json>,
    policy: SpendStoragePolicy,
) -> String {
    let kwargs = RequestKwargs {
        standard_logging_object,
        realtime_tools,
        dynamic_params,
    };
    storage_request_body(params, Some(&kwargs), policy)
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
