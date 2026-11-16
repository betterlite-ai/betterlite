use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::config::ToolPermissionConfigModel;
use betterlite_guardrails::tool_permission::types::{mapping_value, object_list, object_mapping};
use betterlite_guardrails::tool_permission_responses::buffer_overflow::{
    DEFAULT_MAX_STREAM_CHUNKS, StreamBufferLimits,
};
use betterlite_guardrails::tool_permission_responses::error::ResponseToolError;
use betterlite_guardrails::tool_permission_responses::stream_services::StreamServices;
use betterlite_guardrails::tool_permission_responses::streaming::post_call_streaming_iterator_hook;
use betterlite_guardrails::tool_permission_responses::success_hook::{
    ResponseFormat, post_call_success_hook,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const GUARDRAIL_NAME: &str = "tool-permissions";
const DEFAULT_NOW: i64 = 1_700_000_000;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rules: Vec<BTreeMap<String, GuardrailValue>>,
    default_action: String,
    on_disallowed_action: String,
    event_hook: String,
    default_on: bool,
    #[serde(default)]
    violation_message_template: Option<String>,
    mode: String,
    format: String,
    #[serde(default)]
    data: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    response: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    chunks: Vec<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    raw_chunks: Vec<String>,
    #[serde(default)]
    max_chunks: Option<usize>,
}

struct NoCosts;

impl StreamCostCalculator for NoCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

struct NoCounter;

#[async_trait]
impl UsageTokenCounter for NoCounter {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

static NO_COSTS: NoCosts = NoCosts;
static NO_COUNTER: NoCounter = NoCounter;

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn build_guardrail(input: &Input) -> Result<ToolPermissionGuardrail, Box<HelperError>> {
    let settings = ToolPermissionConfigModel {
        optional_params: None,
        rules: None,
        default_action: input.default_action.clone(),
        on_disallowed_action: input.on_disallowed_action.clone(),
    }
    .resolve()
    .or_value_error()?;
    let args = ToolPermissionArgs {
        name: GUARDRAIL_NAME.to_owned(),
        event_hook: LitellmParamsMode::One(input.event_hook.clone()),
        rules: Some(input.rules.clone()),
        default_action: settings.default_action,
        on_disallowed_action: settings.on_disallowed_action,
        default_on: input.default_on,
        violation_message_template: input.violation_message_template.clone(),
    };
    ToolPermissionGuardrail::new(args).or_value_error()
}

fn services(now: i64) -> Result<StreamServices<'static>, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    Ok(StreamServices {
        costs: &NO_COSTS,
        counter: &NO_COUNTER,
        redactor: Arc::new(redactor),
        now,
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}

fn text_of(value: &GuardrailValue) -> Value {
    match value {
        GuardrailValue::Text(text) => Value::String(text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => Value::Null,
    }
}

fn field_text(fields: &Object, key: &str) -> Value {
    fields.get(key).map_or(Value::Null, text_of)
}

fn fields_of(value: Option<&GuardrailValue>) -> Option<&Object> {
    object_mapping(value?)
}

fn tool_call_view(call: &GuardrailValue) -> Value {
    let function = mapping_value(call, "function");
    json!({
        "id": field_text_of(call, "id"),
        "type": field_text_of(call, "type"),
        "function": {
            "name": function.map_or(Value::Null, |function| field_text_of(function, "name")),
            "arguments": function.map_or(Value::Null, |function| field_text_of(function, "arguments")),
        },
    })
}

fn field_text_of(value: &GuardrailValue, key: &str) -> Value {
    mapping_value(value, key).map_or(Value::Null, text_of)
}

fn tool_calls_view(calls: Option<&GuardrailValue>) -> Value {
    let Some(calls) = calls.and_then(object_list) else {
        return Value::Null;
    };
    Value::Array(calls.iter().map(tool_call_view).collect())
}

fn function_view(function: Option<&GuardrailValue>) -> Value {
    let Some(function) = function else {
        return Value::Null;
    };
    json!({
        "name": field_text_of(function, "name"),
        "arguments": field_text_of(function, "arguments"),
    })
}

fn message_view(message: &GuardrailValue) -> Value {
    json!({
        "role": field_text_of(message, "role"),
        "content": field_text_of(message, "content"),
        "tool_calls": tool_calls_view(mapping_value(message, "tool_calls")),
        "function_call": function_view(mapping_value(message, "function_call")),
    })
}

fn choices_view(response: &Object) -> Value {
    let Some(choices) = response.get("choices").and_then(object_list) else {
        return Value::Array(Vec::new());
    };
    let projected: Vec<Value> = choices
        .iter()
        .map(|choice| {
            let message = mapping_value(choice, "message")
                .or_else(|| mapping_value(choice, "delta"))
                .map_or(Value::Null, message_view);
            json!({
                "index": mapping_value(choice, "index").map_or(Value::Null, exact),
                "finish_reason": field_text_of(choice, "finish_reason"),
                "message": message,
            })
        })
        .collect();
    Value::Array(projected)
}

fn exact(value: &GuardrailValue) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn block_view(block: &GuardrailValue) -> Value {
    json!({
        "type": field_text_of(block, "type"),
        "text": field_text_of(block, "text"),
        "id": field_text_of(block, "id"),
        "name": field_text_of(block, "name"),
    })
}

fn anthropic_view(response: &Object) -> Value {
    let content = response
        .get("content")
        .and_then(object_list)
        .map_or_else(Vec::new, |blocks| blocks.iter().map(block_view).collect());
    json!({
        "content": Value::Array(content),
        "stop_reason": field_text(response, "stop_reason"),
    })
}

fn response_view(format: &str, response: &GuardrailValue) -> Result<Value, Box<HelperError>> {
    let Some(fields) = object_mapping(response) else {
        return Ok(Value::Null);
    };
    match format {
        "openai" => Ok(json!({ "choices": choices_view(fields) })),
        "anthropic" => Ok(anthropic_view(fields)),
        other => Err(failure(
            "ValueError",
            format!("unknown response format {other}"),
        )),
    }
}

fn applied_guardrails(data: &Object) -> Option<Vec<String>> {
    let bucket = match data.get("litellm_metadata") {
        Some(value) => value,
        None => data.get("metadata")?,
    };
    let names = mapping_value(bucket, "applied_guardrails")?;
    Some(
        object_list(names)?
            .iter()
            .filter_map(|name| match name {
                GuardrailValue::Text(text) => Some(text.clone()),
                GuardrailValue::Null
                | GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_) => None,
            })
            .collect(),
    )
}

fn blocked_view(error: &GuardrailFailure) -> Option<Value> {
    let GuardrailFailure::GuardrailRaised {
        status_code,
        blocked_content,
        ..
    } = error
    else {
        return None;
    };
    Some(json!({
        "status": status_code,
        "message": error.to_string(),
        "blocked_content": blocked_content,
    }))
}

fn success(input: &Input) -> HelperResult {
    let guardrail = build_guardrail(input)?;
    let format = match input.format.as_str() {
        "openai" => ResponseFormat::OpenAi,
        "anthropic" => ResponseFormat::Anthropic,
        other => {
            return Err(failure(
                "ValueError",
                format!("unknown response format {other}"),
            ));
        }
    };
    let Some(response_fields) = &input.response else {
        return Err(failure(
            "ValueError",
            "success mode needs a response".to_owned(),
        ));
    };
    let mut data = input.data.clone();
    let mut response = GuardrailValue::Object(response_fields.clone());
    let blocked = match post_call_success_hook(
        &guardrail,
        &mut data,
        format,
        &BTreeSet::new(),
        &mut response,
    ) {
        Ok(()) => Value::Null,
        Err(error) => match blocked_view(&error) {
            Some(blocked) => blocked,
            None => return Err(failure("GuardrailFailure", error.to_string())),
        },
    };
    let format_name = input.format.clone();
    let projected = response_view(&format_name, &response)?;
    Ok(json!({
        "blocked": blocked,
        "response": projected,
        "applied_guardrails": applied_guardrails(&data),
    }))
}

fn stream_items(input: &Input) -> Vec<StreamItem> {
    if input.raw_chunks.is_empty() {
        input
            .chunks
            .iter()
            .map(|chunk| StreamItem::Json(GuardrailValue::Object(chunk.clone())))
            .collect()
    } else {
        input
            .raw_chunks
            .iter()
            .map(|chunk| StreamItem::Bytes(chunk.clone().into_bytes()))
            .collect()
    }
}

fn raw_chunk_view(bytes: &[u8]) -> Value {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return json!({ "events": Value::Array(Vec::new()) });
    };
    let events: Vec<Value> = text.split("\n\n").filter_map(event_object).collect();
    json!({ "events": Value::Array(events) })
}

fn event_object(frame: &str) -> Option<Value> {
    let payload = frame
        .lines()
        .find_map(|line| line.trim().strip_prefix("data:"))?
        .trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    serde_json::from_str(payload).ok()
}

fn stream_view(items: &[StreamItem]) -> Value {
    let projected: Vec<Value> = items
        .iter()
        .map(|item| match item {
            StreamItem::Json(value) => {
                let Some(fields) = object_mapping(value) else {
                    return Value::Null;
                };
                json!({ "choices": choices_view(fields) })
            }
            StreamItem::Bytes(bytes) => raw_chunk_view(bytes),
        })
        .collect();
    Value::Array(projected)
}

async fn run_stream_hook(
    guardrail: &ToolPermissionGuardrail,
    limits: StreamBufferLimits,
    services: &StreamServices<'_>,
    items: Vec<StreamItem>,
) -> (Result<(), ResponseToolError>, Vec<StreamItem>) {
    let capacity = items.len().max(1);
    let (sender, receiver) = mpsc::channel(capacity);
    let (output, mut output_receiver) = mpsc::channel(64);
    let feed = async move {
        for item in items {
            if sender.send(item).await.is_err() {
                break;
            }
        }
    };
    let hook = post_call_streaming_iterator_hook(guardrail, limits, services, receiver, output);
    let collect = async {
        let mut chunks = Vec::new();
        while let Some(item) = output_receiver.recv().await {
            chunks.push(item);
        }
        chunks
    };
    let ((), result, collected) = tokio::join!(feed, hook, collect);
    (result, collected)
}

fn streaming(input: &Input) -> HelperResult {
    let guardrail = build_guardrail(input)?;
    let services = services(DEFAULT_NOW)?;
    let limits = StreamBufferLimits {
        max_chunks: input.max_chunks.unwrap_or(DEFAULT_MAX_STREAM_CHUNKS),
    };
    let runtime = runtime()?;
    let (result, collected) = runtime.block_on(run_stream_hook(
        &guardrail,
        limits,
        &services,
        stream_items(input),
    ));
    let blocked = match result {
        Ok(()) => Value::Null,
        Err(ResponseToolError::Guardrail(error)) => match blocked_view(&error) {
            Some(blocked) => blocked,
            None => return Err(failure("GuardrailFailure", error.to_string())),
        },
        Err(other) => return Err(failure("ResponseToolError", other.to_string())),
    };
    Ok(json!({
        "chunks": stream_view(&collected),
        "blocked": blocked,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.mode.as_str() {
            "success" => success(input),
            "streaming" => streaming(input),
            other => Err(failure("ValueError", format!("unknown mode {other}"))),
        }
    }
}
