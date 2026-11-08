mod services;
mod tests;
mod values;

use betterlite_logging::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use betterlite_logging::otel::mappers::genai::GenAIMapper;
use betterlite_logging::otel::mappers::legacy::LegacyMapper;
use betterlite_logging::otel::mappers::utils::{
    MAX_TOOL_ATTRS_PER_SPAN, serialize_messages, tool_attr_budget,
};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::metadata::wire::payload::Payload;
use betterlite_logging::otel::model::payloads::guardrail::GuardrailSpanData;
use betterlite_logging::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use betterlite_logging::otel::model::payloads::mcp_call::{
    MCPListToolsSpanData, MCPToolCallSpanData,
};
use betterlite_logging::otel::model::payloads::service::ServiceSpanData;
use betterlite_logging::otel::model::trace_controls::TraceControls;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::transforms::{transform_messages, transform_responses_output};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use services::service_payload;
use std::collections::BTreeMap;
use values::{attributes_record, helper_error};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "genai_llm_call")]
    GenaiLlmCall(LlmCallInput),
    #[serde(rename = "genai_guardrail")]
    GenaiGuardrail(GuardrailInput),
    #[serde(rename = "genai_service")]
    GenaiService(ServiceInput),
    #[serde(rename = "genai_mcp_tool_call")]
    GenaiMcpToolCall(McpInput),
    #[serde(rename = "genai_mcp_list_tools")]
    GenaiMcpListTools(McpInput),
    #[serde(rename = "legacy_llm_call")]
    LegacyLlmCall(LlmCallInput),
    #[serde(rename = "tool_definition_budget")]
    ToolDefinitionBudget(BudgetInput),
    #[serde(rename = "messages")]
    Messages(MessagesInput),
    #[serde(rename = "responses_api_output")]
    ResponsesApiOutput(ResponsesInput),
}

#[derive(Deserialize)]
pub(crate) struct LlmCallInput {
    payload: Box<Payload>,
    #[serde(default)]
    capture_content: bool,
    time_to_first_chunk_seconds: Option<Decimal>,
    request_route: Option<String>,
    session_id: Option<String>,
    #[serde(default = "default_tool_attr_budget")]
    tool_attr_budget: usize,
}

#[derive(Deserialize)]
pub(crate) struct GuardrailInput {
    entry: MetadataMap,
}

#[derive(Deserialize)]
pub(crate) struct ServiceInput {
    service: String,
    call_type: String,
    caller: Option<String>,
    error: Option<String>,
    #[serde(default)]
    event_metadata: MetadataMap,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct McpInput {
    payload: Box<Payload>,
    #[serde(default)]
    capture_content: bool,
}

#[derive(Clone, Copy)]
enum McpMethod {
    ToolCall,
    ListTools,
}

#[derive(Clone, Copy)]
enum Vocabulary {
    Genai,
    Legacy,
}

#[derive(Deserialize)]
pub(crate) struct BudgetInput {
    payload: Box<Payload>,
    vocabularies: usize,
}

#[derive(Deserialize)]
pub(crate) struct MessagesInput {
    messages: Vec<OtelModelValue>,
}

#[derive(Deserialize)]
pub(crate) struct ResponsesInput {
    output: Vec<OtelModelValue>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::GenaiLlmCall(call) => llm_call_record(call, Vocabulary::Genai)?,
            Input::GenaiGuardrail(guardrail) => guardrail_record(&guardrail.entry),
            Input::GenaiService(service) => service_record(service)?,
            Input::GenaiMcpToolCall(call) => mcp_record(call, McpMethod::ToolCall)?,
            Input::GenaiMcpListTools(list) => mcp_record(list, McpMethod::ListTools)?,
            Input::LegacyLlmCall(call) => llm_call_record(call, Vocabulary::Legacy)?,
            Input::ToolDefinitionBudget(budget_input) => budget_record(budget_input)?,
            Input::Messages(messages) => messages_record(&messages.messages),
            Input::ResponsesApiOutput(responses) => {
                json!({"messages": transform_responses_output(&responses.output)})
            }
        };
        Ok(json!({"result": value}))
    }
}

const fn default_tool_attr_budget() -> usize {
    MAX_TOOL_ATTRS_PER_SPAN
}

fn llm_call_span(
    payload: &Payload,
    options: LLMCallOptions,
) -> Result<LLMCallSpanData, Box<crate::helpers::HelperError>> {
    LLMCallSpanData::from_standard_logging_payload(payload, options)
        .map_err(|error| helper_error("MetadataError", error.to_string()))
}

impl Vocabulary {
    fn attributes(self, data: &SpanData<'_>, tool_attr_budget: usize) -> AttributeMap {
        match self {
            Self::Genai => GenAIMapper { tool_attr_budget }.map(data, &BTreeMap::new()),
            Self::Legacy => LegacyMapper { tool_attr_budget }.map(data, &BTreeMap::new()),
        }
    }
}

fn llm_call_record(
    input: &LlmCallInput,
    vocabulary: Vocabulary,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let options = LLMCallOptions {
        capture_content: input.capture_content,
        time_to_first_chunk_seconds: input.time_to_first_chunk_seconds,
        request_route: input.request_route.clone(),
        trace: TraceControls::default(),
        session_id: input.session_id.clone(),
    };
    let span = llm_call_span(&input.payload, options)?;
    let attributes = vocabulary.attributes(&SpanData::LLMCall(&span), input.tool_attr_budget);
    Ok(attributes_record(&attributes))
}

fn guardrail_record(entry: &MetadataMap) -> Value {
    let span = GuardrailSpanData::from_logging_entry(entry);
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let attributes = mapper.map(&SpanData::Guardrail(&span), &BTreeMap::new());
    attributes_record(&attributes)
}

fn service_record(input: &ServiceInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let payload = service_payload(input)?;
    let span = ServiceSpanData::from_payload(&payload, &input.event_metadata);
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let attributes = mapper.map(&SpanData::Service(&span), &input.environment);
    Ok(attributes_record(&attributes))
}

fn mcp_record(
    input: &McpInput,
    method: McpMethod,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let attributes = match method {
        McpMethod::ToolCall => {
            let span = MCPToolCallSpanData::from_standard_logging_payload(
                &input.payload,
                input.capture_content,
            )
            .map_err(|error| helper_error("MetadataError", error.to_string()))?;
            mapper.map(&SpanData::MCPToolCall(&span), &BTreeMap::new())
        }
        McpMethod::ListTools => {
            let span = MCPListToolsSpanData::from_standard_logging_payload(&input.payload)
                .map_err(|error| helper_error("MetadataError", error.to_string()))?;
            mapper.map(&SpanData::MCPListTools(&span), &BTreeMap::new())
        }
    };
    Ok(attributes_record(&attributes))
}

fn budget_record(input: &BudgetInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(&input.payload, LLMCallOptions::default())?;
    let share = tool_attr_budget(input.vocabularies);
    let genai = GenAIMapper {
        tool_attr_budget: share,
    };
    let legacy = LegacyMapper {
        tool_attr_budget: share,
    };
    Ok(budget_map(
        &genai,
        &legacy,
        &SpanData::LLMCall(&span),
        share,
    ))
}

fn budget_map(
    genai: &GenAIMapper,
    legacy: &LegacyMapper,
    data: &SpanData<'_>,
    share: usize,
) -> Value {
    json!({
        "budget": share,
        "genai": attributes_record(&genai.map(data, &BTreeMap::new())),
        "legacy": attributes_record(&legacy.map(data, &BTreeMap::new())),
    })
}

fn messages_record(messages: &[OtelModelValue]) -> Value {
    let objects: Vec<MetadataMap> = messages
        .iter()
        .filter_map(|message| message.as_object().cloned())
        .collect();
    let serialized =
        serialize_messages(&objects).and_then(|text| serde_json::from_str::<Value>(&text).ok());
    json!({
        "serialized": serialized.unwrap_or(Value::Null),
        "parts": transform_messages(&OtelModelValue::Array(messages.to_vec())),
    })
}
