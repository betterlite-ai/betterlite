use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::mcp_tool_hook::dispatch::async_post_mcp_tool_call_hook;
use betterlite_logging::mcp_tool_hook::types::McpToolResult;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::Registration;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::compare::helper::HelperError;

use super::failure;
use super::input::Input;
use super::logs::ErrorLines;
use super::scripted::ScriptedLogger;

const START_TIME: &str = "2026-01-01T00:00:00+00:00";
const END_TIME: &str = "2026-01-01T00:00:01+00:00";

pub(super) async fn execute(input: &Input, logs: &ErrorLines) -> crate::helpers::HelperResult {
    let registry = registry_for(input)?;
    let start_time =
        DateTime::parse_from_rfc3339(START_TIME).map_err(|error| failure(error.to_string()))?;
    let end_time =
        DateTime::parse_from_rfc3339(END_TIME).map_err(|error| failure(error.to_string()))?;
    let mut state = state(input, start_time)?;
    let result = McpToolResult {
        content: input.result.content.clone(),
        structured_content: input.result.structured_content.clone(),
        is_error: input.result.is_error,
    };
    state
        .model_call_details
        .insert("original_response".to_owned(), original_response(&result));
    let final_result =
        async_post_mcp_tool_call_hook(&mut state, &registry, result, start_time, end_time).await;
    output(&final_result, &state, logs)
}

fn registry_for(input: &Input) -> Result<CallbackRegistry, Box<HelperError>> {
    let mut registry = CallbackRegistry::new(input.global_success.len().saturating_add(1));
    for wire in &input.global_success {
        let logger: Arc<dyn CallbackLogger> = Arc::new(ScriptedLogger { wire: wire.clone() });
        let identity = CallbackIdentity {
            class_name: wire.name.clone(),
            attributes: IndexMap::new(),
        };
        let registration = registry.register(
            identity,
            CallbackSink::Logger(logger),
            CallbackList::Success,
        );
        if registration != Registration::Registered {
            return Err(failure(format!(
                "the logger {} was not registered",
                wire.name
            )));
        }
    }
    Ok(registry)
}

fn state(
    input: &Input,
    start_time: DateTime<FixedOffset>,
) -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "call_mcp_tool".to_owned(),
        start_time,
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
        messages: None,
        stream: None,
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks {
            input: None,
            success: (!input.dynamic_success.is_empty()).then(|| input.dynamic_success.clone()),
            async_success: None,
            failure: None,
            async_failure: None,
        },
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| failure(error.to_string()))
}

fn original_response(result: &McpToolResult) -> Json {
    Json::Object(IndexMap::from([
        ("content".to_owned(), Json::List(result.content.clone())),
        (
            "structured_content".to_owned(),
            result.structured_content.clone().unwrap_or(Json::Null),
        ),
        ("is_error".to_owned(), Json::Bool(result.is_error)),
    ]))
}

fn output(
    result: &McpToolResult,
    state: &LoggingState,
    logs: &ErrorLines,
) -> crate::helpers::HelperResult {
    let mut fields = serde_json::Map::new();
    let content =
        serde_json::to_value(&result.content).map_err(|error| failure(error.to_string()))?;
    let structured = serde_json::to_value(&result.structured_content)
        .map_err(|error| failure(error.to_string()))?;
    fields.insert("content".to_owned(), content);
    fields.insert("structured_content".to_owned(), structured);
    fields.insert(
        "is_error".to_owned(),
        serde_json::Value::Bool(result.is_error),
    );
    fields.insert(
        "error_lines".to_owned(),
        serde_json::Value::from(logs.0.load(Ordering::Relaxed)),
    );
    if let Some(cost) = state.model_call_details.get("response_cost") {
        let encoded = serde_json::to_value(cost).map_err(|error| failure(error.to_string()))?;
        fields.insert("response_cost".to_owned(), encoded);
    }
    Ok(serde_json::Value::Object(fields))
}
