mod tests;
mod values;

use betterlite_logging::otel::mappers::base::{AttributeMapper as _, SpanData};
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
use betterlite_logging::otel::model::payloads::service::{
    ServiceLoggerPayload, ServiceSpanData, ServiceType,
};
use betterlite_logging::otel::model::trace_controls::TraceControls;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::transforms::{transform_messages, transform_responses_output};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use values::{attributes_record, invalid_input, metadata_error};

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
    GenaiMcpToolCall(ToolCallInput),
    #[serde(rename = "genai_mcp_list_tools")]
    GenaiMcpListTools(ListToolsInput),
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
    tool_attr_budget: Option<usize>,
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
pub(crate) struct ToolCallInput {
    payload: Box<Payload>,
    #[serde(default)]
    capture_content: bool,
}

#[derive(Deserialize)]
pub(crate) struct ListToolsInput {
    payload: Box<Payload>,
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
            Input::GenaiLlmCall(call) => genai_llm_call(call)?,
            Input::GenaiGuardrail(guardrail) => guardrail_record(&guardrail.entry),
            Input::GenaiService(service) => service_record(service)?,
            Input::GenaiMcpToolCall(call) => tool_call_record(call)?,
            Input::GenaiMcpListTools(list) => list_tools_record(&list.payload)?,
            Input::LegacyLlmCall(call) => legacy_llm_call(call)?,
            Input::ToolDefinitionBudget(budget_input) => budget_record(budget_input)?,
            Input::Messages(messages) => messages_record(&messages.messages),
            Input::ResponsesApiOutput(responses) => {
                json!({"messages": transform_responses_output(&responses.output)})
            }
        };
        Ok(json!({"result": value}))
    }
}

const fn budget(value: Option<usize>) -> usize {
    match value {
        Some(share) => share,
        None => MAX_TOOL_ATTRS_PER_SPAN,
    }
}

fn llm_call_span(
    payload: &Payload,
    options: LLMCallOptions,
) -> Result<LLMCallSpanData, Box<crate::helpers::HelperError>> {
    LLMCallSpanData::from_standard_logging_payload(payload, options)
        .map_err(|error| metadata_error(&error))
}

fn genai_llm_call(input: &LlmCallInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(
        &input.payload,
        LLMCallOptions {
            capture_content: input.capture_content,
            time_to_first_chunk_seconds: input.time_to_first_chunk_seconds,
            request_route: input.request_route.clone(),
            trace: TraceControls::default(),
            session_id: input.session_id.clone(),
        },
    )?;
    let mapper = GenAIMapper {
        tool_attr_budget: budget(input.tool_attr_budget),
    };
    Ok(attributes_record(
        &mapper.map(&SpanData::LLMCall(&span), &BTreeMap::new()),
    ))
}

fn legacy_llm_call(input: &LlmCallInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(&input.payload, LLMCallOptions::default())?;
    let mapper = LegacyMapper {
        tool_attr_budget: budget(input.tool_attr_budget),
    };
    Ok(attributes_record(
        &mapper.map(&SpanData::LLMCall(&span), &BTreeMap::new()),
    ))
}

fn guardrail_record(entry: &MetadataMap) -> Value {
    let span = GuardrailSpanData::from_logging_entry(entry);
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    attributes_record(&mapper.map(&SpanData::Guardrail(&span), &BTreeMap::new()))
}

fn service_record(input: &ServiceInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let payload = ServiceLoggerPayload {
        service: service_type(&input.service)?,
        call_type: input.call_type.clone(),
        caller: input.caller.clone(),
        error: input.error.clone(),
    };
    let span = ServiceSpanData::from_payload(&payload, &input.event_metadata);
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    Ok(attributes_record(
        &mapper.map(&SpanData::Service(&span), &input.environment),
    ))
}

fn service_type(name: &str) -> Result<ServiceType, Box<crate::helpers::HelperError>> {
    match name {
        "redis" => Ok(ServiceType::Redis),
        "postgres" => Ok(ServiceType::Postgres),
        "batch_write_to_db" => Ok(ServiceType::BatchWriteToDb),
        "reset_budget_job" => Ok(ServiceType::ResetBudgetJob),
        "self" => Ok(ServiceType::SelfService),
        "router" => Ok(ServiceType::Router),
        "auth" => Ok(ServiceType::Auth),
        "proxy_pre_call" => Ok(ServiceType::ProxyPreCall),
        "pod_lock_manager" => Ok(ServiceType::PodLockManager),
        "in_memory_daily_spend_update_queue" => Ok(ServiceType::InMemoryDailySpendUpdateQueue),
        "redis_daily_spend_update_queue" => Ok(ServiceType::RedisDailySpendUpdateQueue),
        "redis_daily_end_user_spend_update_queue" => Ok(ServiceType::RedisDailyEndUserQueue),
        "redis_daily_org_spend_update_queue" => Ok(ServiceType::RedisDailyOrgSpendUpdateQueue),
        "redis_daily_team_spend_update_queue" => Ok(ServiceType::RedisDailyTeamSpendUpdateQueue),
        "redis_daily_agent_spend_update_queue" => Ok(ServiceType::RedisDailyAgentQueue),
        "redis_daily_tag_spend_update_queue" => Ok(ServiceType::RedisDailyTagSpendUpdateQueue),
        "in_memory_spend_update_queue" => Ok(ServiceType::InMemorySpendUpdateQueue),
        "redis_spend_update_queue" => Ok(ServiceType::RedisSpendUpdateQueue),
        "redis_window_spend_update_queue" => Ok(ServiceType::RedisWindowSpendUpdateQueue),
        other => Err(invalid_input(format!("unknown service {other}"))),
    }
}

fn tool_call_record(input: &ToolCallInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span =
        MCPToolCallSpanData::from_standard_logging_payload(&input.payload, input.capture_content)
            .map_err(|error| metadata_error(&error))?;
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    Ok(attributes_record(
        &mapper.map(&SpanData::MCPToolCall(&span), &BTreeMap::new()),
    ))
}

fn list_tools_record(payload: &Payload) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = MCPListToolsSpanData::from_standard_logging_payload(payload)
        .map_err(|error| metadata_error(&error))?;
    let mapper = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    Ok(attributes_record(
        &mapper.map(&SpanData::MCPListTools(&span), &BTreeMap::new()),
    ))
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
    Ok(json!({
        "budget": share,
        "genai": attributes_record(&genai.map(&SpanData::LLMCall(&span), &BTreeMap::new())),
        "legacy": attributes_record(&legacy.map(&SpanData::LLMCall(&span), &BTreeMap::new())),
    }))
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
