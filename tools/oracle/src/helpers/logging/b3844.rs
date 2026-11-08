use betterlite_logging::otel::model::metadata::wire::MetadataError;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::metadata::wire::payload::Payload;
use betterlite_logging::otel::model::payloads::RequestIdentity;
use betterlite_logging::otel::model::payloads::llm_call::{
    EmbeddingOutput, LLMCallOptions, LLMCallSpanData,
};
use betterlite_logging::otel::model::payloads::mcp_call::{
    MCPListToolsSpanData, MCPToolCallSpanData, PayloadKind,
};
use betterlite_logging::otel::model::payloads::tool_definition::extract_tools;
use betterlite_logging::otel::model::payloads::usage::LLMUsage;
use betterlite_logging::otel::model::trace_controls::TraceControls;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::span_attributes::primitive_value;
use opentelemetry::Value as OtelValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    PrimitiveValue {
        value: OtelModelValue,
    },
    Tools {
        value: MetadataMap,
    },
    EmbeddingOutput {
        value: MetadataMap,
    },
    LlmCall {
        payload: Box<Payload>,
        #[serde(default)]
        capture_content: bool,
        time_to_first_chunk_seconds: Option<Decimal>,
        request_route: Option<String>,
        session_id: Option<String>,
    },
    McpToolCall {
        payload: Box<Payload>,
        #[serde(default)]
        capture_content: bool,
    },
    McpListTools {
        payload: Box<Payload>,
    },
    PayloadKind {
        payload: Box<Payload>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::PrimitiveValue { value } => primitive_record(value),
            Input::Tools { value } => json!(extract_tools(value)),
            Input::EmbeddingOutput { value } => json!(EmbeddingOutput::from_response(value)),
            Input::LlmCall {
                payload,
                capture_content,
                time_to_first_chunk_seconds,
                request_route,
                session_id,
            } => llm_call(
                payload,
                *capture_content,
                *time_to_first_chunk_seconds,
                request_route.as_deref(),
                session_id.as_deref(),
            )?,
            Input::McpToolCall {
                payload,
                capture_content,
            } => tool_call(payload, *capture_content)?,
            Input::McpListTools { payload } => list_tools(payload)?,
            Input::PayloadKind { payload } => kind_record(payload),
        };
        Ok(json!({"result": value}))
    }
}

fn llm_call(
    payload: &Payload,
    capture_content: bool,
    time_to_first_chunk_seconds: Option<Decimal>,
    request_route: Option<&str>,
    session_id: Option<&str>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = LLMCallSpanData::from_standard_logging_payload(
        payload,
        LLMCallOptions {
            capture_content,
            time_to_first_chunk_seconds,
            request_route: request_route.map(str::to_owned),
            trace: TraceControls::default(),
            session_id: session_id.map(str::to_owned),
        },
    )
    .map_err(|error| metadata_error(&error))?;
    Ok(llm_call_record(&span))
}

fn tool_call(
    payload: &Payload,
    capture_content: bool,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = MCPToolCallSpanData::from_standard_logging_payload(payload, capture_content)
        .map_err(|error| metadata_error(&error))?;
    Ok(tool_call_record(&span))
}

fn list_tools(payload: &Payload) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = MCPListToolsSpanData::from_standard_logging_payload(payload)
        .map_err(|error| metadata_error(&error))?;
    Ok(list_tools_record(&span))
}

fn metadata_error(error: &MetadataError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "MetadataError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn primitive_record(value: &OtelModelValue) -> Value {
    match primitive_value(value) {
        OtelValue::Bool(flag) => json!(flag),
        OtelValue::I64(integer) => json!(integer),
        OtelValue::F64(float) => json!(float),
        OtelValue::String(text) => json!(text.to_string()),
        OtelValue::Array(_) | _ => json!(""),
    }
}

fn llm_call_record(span: &LLMCallSpanData) -> Value {
    json!({
        "operation": span.operation.to_string(),
        "provider": span.provider,
        "request_model": span.request_model,
        "response_model": span.response_model,
        "response_id": span.response_id,
        "request_params": span.request_params,
        "usage": usage_record(&span.usage),
        "finish_reasons": span.finish_reasons,
        "error": span.error,
        "response_cost": decimal_value(span.response_cost),
        "server": span.server,
        "identity": identity_record(&span.identity),
        "is_streaming": span.is_streaming,
        "cost": span.cost,
        "tools": span.tools,
        "messages_in": span.messages_in,
        "choices_out": span.choices_out,
        "system_fingerprint": span.system_fingerprint,
        "time_to_first_chunk_seconds": decimal_value(span.time_to_first_chunk_seconds),
        "output_type": span.output_type.map(|output| output.to_string()),
        "call_type": span.call_type,
        "request_route": span.request_route,
        "session_id": span.session_id,
        "embedding_output": span.embedding_output,
    })
}

fn tool_call_record(span: &MCPToolCallSpanData) -> Value {
    json!({
        "operation": span.operation.to_string(),
        "method": span.method.to_string(),
        "tool_name": span.tool_name,
        "server_name": span.server_name,
        "server_address": span.server_address,
        "server_port": span.server_port,
        "session_id": span.session_id,
        "arguments_json": span.arguments_json,
        "result_json": span.result_json,
        "error": span.error,
        "response_cost": decimal_value(span.response_cost),
        "identity": identity_record(&span.identity),
    })
}

fn list_tools_record(span: &MCPListToolsSpanData) -> Value {
    json!({
        "method": span.method.to_string(),
        "session_id": span.session_id,
        "error": span.error,
        "identity": identity_record(&span.identity),
    })
}

fn kind_record(payload: &Payload) -> Value {
    match PayloadKind::from_payload(payload) {
        PayloadKind::McpToolCall => json!("mcp_tool_call"),
        PayloadKind::McpListTools => json!("mcp_list_tools"),
        PayloadKind::Inference => json!("inference"),
    }
}

fn usage_record(usage: &LLMUsage) -> Value {
    json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "total_tokens": usage.total_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "reasoning_tokens": usage.reasoning_tokens,
        "uncached_input_tokens": usage.uncached_input_tokens(),
        "non_reasoning_output_tokens": usage.non_reasoning_output_tokens(),
    })
}

fn identity_record(identity: &RequestIdentity) -> Value {
    json!({
        "call_id": identity.call_id,
        "request_route": identity.request_route,
    })
}

fn decimal_value(value: Option<Decimal>) -> Value {
    value
        .and_then(|amount| amount.to_f64())
        .map_or(Value::Null, |float| json!(float))
}
