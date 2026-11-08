use super::super::metadata::wire::{
    MetadataError, MetadataMap,
    payload::{Payload, Response},
};
use super::super::trace_controls::TraceControls;
use super::super::utils::coercion::OtelModelValue;
use super::{
    RequestContext, RequestIdentity,
    cost::LLMCost,
    error::SpanError,
    helpers,
    output_choices::{finish_reasons, output_choices},
    request_params::LLMRequestParams,
    server::ServerInfo,
    tool_definition::{ToolDefinition, extract_tools},
    usage::LLMUsage,
};
use crate::otel::vocabulary::constants::{GenAIOperation, GenAIOutputType};
use crate::otel::vocabulary::resolvers::{
    resolve_operation, resolve_output_type, resolve_provider,
};
use rust_decimal::Decimal;

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct EmbeddingOutput {
    pub count: usize,
    pub dimensions: Option<usize>,
}

impl EmbeddingOutput {
    #[must_use]
    pub fn from_response(response: &MetadataMap) -> Option<Self> {
        let rows = helpers::dicts(response.get("data")?);
        let first = rows.first()?;
        // a base64 vector arrives as one string, so only an array has a width
        let dimensions = first
            .get("embedding")
            .and_then(OtelModelValue::as_array)
            .map(<[OtelModelValue]>::len);
        Some(Self {
            count: rows.len(),
            dimensions,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct LLMCallOptions {
    pub capture_content: bool,
    pub time_to_first_chunk_seconds: Option<Decimal>,
    pub request_route: Option<String>,
    pub trace: TraceControls,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LLMCallSpanData {
    pub operation: GenAIOperation,
    pub provider: String,
    pub request_model: String,
    pub response_model: Option<String>,
    pub response_id: Option<String>,
    pub request_params: LLMRequestParams,
    pub usage: LLMUsage,
    pub finish_reasons: Vec<String>,
    pub error: Option<SpanError>,
    pub response_cost: Option<Decimal>,
    pub server: Option<ServerInfo>,
    pub identity: RequestIdentity,
    pub is_streaming: Option<bool>,
    pub cost: LLMCost,
    pub tools: Vec<ToolDefinition>,
    pub messages_in: Vec<MetadataMap>,
    pub choices_out: Vec<MetadataMap>,
    pub system_fingerprint: Option<String>,
    pub time_to_first_chunk_seconds: Option<Decimal>,
    pub output_type: Option<GenAIOutputType>,
    pub call_type: Option<String>,
    pub request_route: Option<String>,
    pub trace: TraceControls,
    pub session_id: Option<String>,
    pub embedding_output: Option<EmbeddingOutput>,
}

struct CallFacts {
    context: RequestContext,
    flat: MetadataMap,
    response: MetadataMap,
    choices: Vec<MetadataMap>,
    parameters: MetadataMap,
    cost_breakdown: MetadataMap,
    operation: GenAIOperation,
}

impl CallFacts {
    fn load(payload: &Payload) -> Result<Self, MetadataError> {
        let response = response_fields(&payload.response);
        let choices = output_choices(&response);
        let flat: MetadataMap = serde_json::from_value(serde_json::to_value(payload)?)?;
        Ok(Self {
            operation: resolve_operation(flat_text(&flat, "call_type")),
            context: RequestContext::from_standard_logging_payload(payload)?,
            parameters: object_field(&flat, "model_parameters"),
            cost_breakdown: object_field(&flat, "cost_breakdown"),
            flat,
            response,
            choices,
        })
    }

    fn messages_in(&self) -> Vec<MetadataMap> {
        self.flat
            .get("messages")
            .map_or_else(Vec::new, helpers::dicts)
    }

    fn embedding_output(&self) -> Option<EmbeddingOutput> {
        if self.operation != GenAIOperation::Embeddings {
            return None;
        }
        EmbeddingOutput::from_response(&self.response)
    }
}

impl LLMCallSpanData {
    pub fn from_standard_logging_payload(
        payload: &Payload,
        options: LLMCallOptions,
    ) -> Result<Self, MetadataError> {
        let capture_content = options.capture_content;
        let facts = CallFacts::load(payload)?;
        let mut span = Self::without_content(payload, &facts, options);
        if capture_content {
            span.messages_in = facts.messages_in();
            span.embedding_output = facts.embedding_output();
            span.choices_out = facts.choices;
        }
        Ok(span)
    }

    fn without_content(payload: &Payload, facts: &CallFacts, options: LLMCallOptions) -> Self {
        let call_type = flat_text(&facts.flat, "call_type");
        Self {
            operation: facts.operation,
            provider: resolve_provider(flat_text(&facts.flat, "custom_llm_provider")).to_owned(),
            request_model: facts.context.request_model.clone(),
            response_model: facts.context.response_model.clone(),
            response_id: response_text(&facts.response, "id"),
            request_params: LLMRequestParams::from_model_parameters(&facts.parameters),
            usage: LLMUsage::from_payload(&facts.flat),
            finish_reasons: finish_reasons(&facts.choices),
            error: SpanError::from_payload(&payload.extra),
            response_cost: payload.response_cost,
            server: ServerInfo::from_api_base(facts.context.api_base.as_deref()),
            identity: facts.context.identity.clone(),
            is_streaming: facts.flat.get("stream").and_then(OtelModelValue::as_bool),
            cost: LLMCost::from_breakdown(&facts.cost_breakdown),
            tools: extract_tools(&facts.parameters),
            messages_in: Vec::new(),
            choices_out: Vec::new(),
            system_fingerprint: response_text(&facts.response, "system_fingerprint"),
            time_to_first_chunk_seconds: options.time_to_first_chunk_seconds,
            output_type: resolve_output_type(call_type),
            call_type: non_empty(Some(call_type)),
            request_route: non_empty(options.request_route.as_deref())
                .or_else(|| facts.context.identity.request_route.clone()),
            trace: options.trace,
            session_id: non_empty(options.session_id.as_deref()),
            embedding_output: None,
        }
    }
}

fn response_fields(response: &Response) -> MetadataMap {
    match response {
        Response::Object { extra, .. } => extra.clone(),
        Response::Text(_) | Response::Items(_) | Response::Null => MetadataMap::new(),
    }
}

fn flat_text<'fields>(fields: &'fields MetadataMap, key: &str) -> &'fields str {
    fields
        .get(key)
        .and_then(OtelModelValue::as_str)
        .unwrap_or_default()
}

fn object_field(fields: &MetadataMap, key: &str) -> MetadataMap {
    fields
        .get(key)
        .and_then(OtelModelValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn response_text(response: &MetadataMap, key: &str) -> Option<String> {
    response
        .get(key)
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        Some(_) | None => None,
    }
}

#[path = "llm_call/tests.rs"]
mod tests;
