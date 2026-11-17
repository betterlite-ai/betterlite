#![cfg(test)]

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use tracing::field::Field as EventField;
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::api_boundaries::ApiLimits;
use crate::api::response_chat_result::error::ResponseChatResultError;
use crate::api::response_chat_result::records::ModelResponse;
use crate::api::response_chat_result::transform::{ResponseSource, transform_response};

pub(super) const MODEL: &str = "caller-model";

pub(super) fn output_text(content: &str) -> Value {
    json!({"type": "output_text", "text": content, "annotations": []})
}

pub(super) fn chat_message(content: &str, role: &str) -> Value {
    json!({
        "content": content,
        "function_call": null,
        "provider_specific_fields": null,
        "role": role,
        "tool_calls": null,
    })
}

pub(super) fn text_choice(content: &str, role: &str, index: usize) -> Value {
    json!({"finish_reason": "stop", "index": index, "message": chat_message(content, role)})
}

pub(super) fn summary_block(content: &str) -> Value {
    json!({"type": "summary_text", "text": content})
}

pub(super) fn reasoning_record(id: &str, blocks: Value) -> Value {
    json!({"type": "reasoning", "id": id, "encrypted_content": null, "summary": blocks})
}

pub(super) fn reasoning_item(id: &str, blocks: Value) -> Value {
    json!({"type": "reasoning", "id": id, "summary": blocks})
}

pub(super) fn envelope(output: Value) -> Value {
    json!({
        "id": "resp_plain",
        "created_at": 1_700_000_000_i64,
        "output": output,
        "status": "completed",
    })
}

pub(super) fn decoded(response: Value) -> ResponsesApiResponse {
    serde_json::from_value(response).expect("the fixture decodes")
}

pub(super) fn decoded_with(output: Value, overrides: Value) -> ResponsesApiResponse {
    let mut response = envelope(output);
    let Value::Object(fields) = overrides else {
        panic!("the overrides are an object")
    };
    for (key, value) in fields {
        response[key.as_str()] = value;
    }
    decoded(response)
}

pub(super) fn object(value: Value) -> Map<String, Value> {
    let Value::Object(fields) = value else {
        panic!("the fixture is an object")
    };
    fields
}

pub(super) fn state() -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "responses".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .expect("the fixture is RFC 3339"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
        messages: None,
        stream: Some(true),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
        || "trace-1".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}

// the recorded SSE text a recovering response keeps, as LiteLLM logs it
pub(super) fn recording(output: &str) -> String {
    format!("data: {{\"type\":\"response.completed\",\"response\":{{\"output\":{output}}}}}")
}

pub(super) fn recorded_state(output: &str) -> LoggingState {
    let mut logging = state();
    logging.model_call_details = IndexMap::from([(
        "original_response".to_owned(),
        Json::Text(recording(output)),
    )]);
    logging
}

pub(super) fn caller() -> ModelResponse {
    ModelResponse {
        id: String::new(),
        created: 5_i64,
        model: None,
        object: "chat.completion".to_owned(),
        system_fingerprint: Some("fp".to_owned()),
        choices: Vec::new(),
        usage: None,
        extra: Map::new(),
        hidden_params: Map::new(),
    }
}

pub(super) fn caller_with_hidden(hidden: Value) -> ModelResponse {
    ModelResponse {
        hidden_params: object(hidden),
        ..caller()
    }
}

pub(super) fn run(
    response: &mut ResponsesApiResponse,
    logging: &LoggingState,
) -> (Result<ModelResponse, ResponseChatResultError>, Vec<String>) {
    run_with(caller(), response, &Map::new(), logging)
}

pub(super) fn run_with(
    result: ModelResponse,
    response: &mut ResponsesApiResponse,
    hidden: &Map<String, Value>,
    logging: &LoggingState,
) -> (Result<ModelResponse, ResponseChatResultError>, Vec<String>) {
    capture(|| {
        let source = ResponseSource {
            model: MODEL,
            response,
            hidden_params: hidden,
        };
        transform_response(source, result, logging, &ApiLimits::default())
    })
}

pub(super) fn choices_value(record: &ModelResponse) -> Value {
    serde_json::to_value(&record.choices).expect("the choices serialize")
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |_field: &EventField, value: &dyn fmt::Debug| {
            message = format!("{value:?}");
        });
        self.0.lock().unwrap().push(message);
    }
}

pub(super) fn capture<Output, Call>(call: Call) -> (Output, Vec<String>)
where
    Call: FnOnce() -> Output,
{
    let captured = Captured::default();
    let subscriber = registry().with(captured.clone());
    let output = with_default(subscriber, call);
    let lines = captured.0.lock().unwrap().clone();
    (output, lines)
}
