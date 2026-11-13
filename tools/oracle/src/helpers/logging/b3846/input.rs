use betterlite_logging::otel::mappers::utils::MAX_TOOL_ATTRS_PER_SPAN;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::metadata::wire::payload::Payload;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "langfuse_llm_call")]
    LangfuseLlmCall(LlmCallInput),
    #[serde(rename = "langtrace_llm_call")]
    LangtraceLlmCall(LlmCallInput),
    #[serde(rename = "weave_llm_call")]
    WeaveLlmCall(LlmCallInput),
    #[serde(rename = "openinference_llm_call")]
    OpenInferenceLlmCall(LlmCallInput),
    #[serde(rename = "openinference_message_shedding")]
    OpenInferenceMessageShedding(SheddingInput),
    #[serde(rename = "mapper_registry")]
    MapperRegistry { names: Vec<String> },
    #[serde(rename = "langtrace_integration")]
    LangtraceIntegration {
        kwargs: MetadataMap,
        response: MetadataMap,
    },
    #[serde(rename = "infer_span_kind")]
    InferSpanKind { call_type: Option<String> },
    #[serde(rename = "tool_attributes")]
    ToolAttributes {
        optional_tools: Option<Vec<OtelModelValue>>,
        metadata_tools: Option<Vec<OtelModelValue>>,
    },
}

#[derive(Deserialize)]
pub(crate) struct LlmCallInput {
    pub(crate) payload: Box<Payload>,
    #[serde(default)]
    pub(crate) capture_content: bool,
    pub(crate) time_to_first_chunk_seconds: Option<Decimal>,
    pub(crate) request_route: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) trace: Option<TraceInput>,
    #[serde(default = "default_tool_attr_budget")]
    pub(crate) tool_attr_budget: usize,
}

#[derive(Deserialize)]
pub(crate) struct TraceInput {
    pub(crate) name: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct SheddingInput {
    pub(crate) budget: Option<usize>,
    #[serde(flatten)]
    pub(crate) call: LlmCallInput,
}

const fn default_tool_attr_budget() -> usize {
    MAX_TOOL_ATTRS_PER_SPAN
}
