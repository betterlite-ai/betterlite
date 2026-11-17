use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_pre_post::hooks::PreCallEvent;
use betterlite_logging::otel::logger::pre_call::log_pre_api_call;
use betterlite_logging::otel::logger::v2::OpenTelemetryV2;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::harness::HarnessError;

pub(super) const MODEL: &str = "gpt-4o";
pub(super) const CALL_ID: &str = "call-1";

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "litellm_call_id": "call-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub(super) fn text_object(params: &BTreeMap<String, String>) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(key, value)| (key.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

pub(super) fn space_params(space: &str) -> Json {
    Json::Object(IndexMap::from([(
        "arize_space_id".to_owned(),
        Json::Text(space.to_owned()),
    )]))
}

pub(super) fn pre_call_details(
    credentials: Option<&BTreeMap<String, String>>,
    auth_metadata: Option<&BTreeMap<String, String>>,
) -> Json {
    let mut details = IndexMap::from([
        ("model".to_owned(), Json::Text(MODEL.to_owned())),
        ("call_type".to_owned(), Json::Text("completion".to_owned())),
        ("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned())),
    ]);
    if let Some(dynamic_params) = credentials {
        details.insert(
            "standard_callback_dynamic_params".to_owned(),
            text_object(dynamic_params),
        );
    }
    if let Some(identity) = auth_metadata {
        details.insert(
            "litellm_params".to_owned(),
            Json::Object(IndexMap::from([(
                "metadata".to_owned(),
                Json::Object(IndexMap::from([(
                    "user_api_key_auth_metadata".to_owned(),
                    text_object(identity),
                )])),
            )])),
        );
    }
    Json::Object(details)
}

// litellm.integrations.otel.logger.OpenTelemetryV2.log_pre_api_call, as the proxy calls it
pub(super) fn boundary(
    logger: &OpenTelemetryV2,
    credentials: Option<&BTreeMap<String, String>>,
    auth_metadata: Option<&BTreeMap<String, String>>,
) -> Result<(), Box<HelperError>> {
    let details = pre_call_details(credentials, auth_metadata);
    let messages = Json::List(Vec::new());
    log_pre_api_call(
        logger,
        &PreCallEvent {
            model: MODEL,
            messages: &messages,
            details: &details,
        },
    )
    .or_value_error()
}

pub(super) fn payload_metadata(
    auth_metadata: Option<&BTreeMap<String, String>>,
    tool_name: Option<&str>,
) -> Value {
    let mut metadata = serde_json::Map::new();
    if let Some(name) = tool_name {
        metadata.insert("mcp_tool_call_metadata".to_owned(), json!({"name": name}));
    }
    if let Some(identity) = auth_metadata {
        metadata.insert("user_api_key_auth_metadata".to_owned(), json!(identity));
    }
    Value::Object(metadata)
}

pub(super) fn payload_document(
    call_type: &str,
    status: &str,
    metadata: Value,
) -> Result<StandardLoggingPayload, Box<HelperError>> {
    let mut value: Value = serde_json::from_str(PAYLOAD)?;
    let object = value
        .as_object_mut()
        .ok_or(HarnessError::Fixture)
        .or_value_error()?;
    object.insert("call_type".to_owned(), Value::String(call_type.to_owned()));
    object.insert("status".to_owned(), Value::String(status.to_owned()));
    if status == "failure" {
        object.insert(
            "status_fields".to_owned(),
            json!({"llm_api_status": "failure", "guardrail_status": "not_run"}),
        );
    }
    object.insert("metadata".to_owned(), metadata);
    Ok(serde_json::from_value(value)?)
}

pub(super) fn event(payload: StandardLoggingPayload, dynamic: Json) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn deliver(
    logger: &OpenTelemetryV2,
    event: Arc<LoggerEvent>,
    failure: bool,
) -> Result<(), Box<HelperError>> {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    runtime.block_on(async {
        if failure {
            logger.async_log_failure_event(event).await
        } else {
            logger.async_log_success_event(event).await
        }
        .or_value_error()
    })
}
