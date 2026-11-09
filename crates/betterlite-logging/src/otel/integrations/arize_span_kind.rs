use opentelemetry::KeyValue;
use opentelemetry::trace::Span;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

const LLM_TOOLS: &str = "llm.tools";
const METADATA_TOOL_PREFIX: &str = "llm.invocation_parameters.tools";

// passthrough is the spelling real call types use: allm_passthrough_route
#[rustfmt::skip]
const LLM_KEYWORDS: [&str; 14] = [
    "completion", "chat", "image", "audio", "speech", "transcription", "generate_content",
    "response", "videos", "realtime", "pass_through", "passthrough", "anthropic_messages", "ocr",
];

const CHAIN_KEYWORDS: [&str; 4] = ["file", "batch", "container", "fine_tuning_job"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum OpenInferenceSpanKind {
    Tool,
    Chain,
    Llm,
    Retriever,
    Embedding,
    Agent,
    Reranker,
    Unknown,
    Guardrail,
}

#[must_use]
pub fn infer_open_inference_span_kind(call_type: Option<&str>) -> OpenInferenceSpanKind {
    let Some(declared) = call_type.filter(|value| !value.is_empty()) else {
        return OpenInferenceSpanKind::Unknown;
    };
    let lowered = declared.to_lowercase();
    if lowered.contains("embed") {
        return OpenInferenceSpanKind::Embedding;
    }
    if lowered.contains("rerank") {
        return OpenInferenceSpanKind::Reranker;
    }
    if lowered.contains("search") {
        return OpenInferenceSpanKind::Retriever;
    }
    if lowered.contains("moderation") || lowered.contains("guardrail") {
        return OpenInferenceSpanKind::Guardrail;
    }
    if lowered == "call_mcp_tool" || lowered == "mcp" || lowered.ends_with("tool") {
        return OpenInferenceSpanKind::Tool;
    }
    if lowered.contains("asend_message") || lowered.contains("a2a") || lowered.contains("assistant")
    {
        return OpenInferenceSpanKind::Agent;
    }
    if LLM_KEYWORDS.iter().any(|keyword| lowered.contains(keyword)) {
        return OpenInferenceSpanKind::Llm;
    }
    if CHAIN_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
    {
        return OpenInferenceSpanKind::Chain;
    }
    OpenInferenceSpanKind::Unknown
}

pub fn set_tool_attributes<S>(
    span: &mut S,
    optional_tools: Option<&[OtelModelValue]>,
    metadata_tools: Option<&[OtelModelValue]>,
) where
    S: Span,
{
    if let Some(tools) = optional_tools {
        for (index, tool) in tools.iter().enumerate() {
            let Some(fields) = tool.as_object() else {
                continue;
            };
            let Some(function) = fields.get("function").and_then(OtelModelValue::as_object) else {
                continue;
            };
            emit_optional_tool(span, index, function);
        }
    }
    if let Some(tools) = metadata_tools {
        for (index, tool) in tools.iter().enumerate() {
            let Some(fields) = tool.as_object() else {
                continue;
            };
            emit_name_and_description(span, METADATA_TOOL_PREFIX, index, fields);
        }
    }
}

fn emit_name_and_description<S>(span: &mut S, prefix: &str, index: usize, fields: &MetadataMap)
where
    S: Span,
{
    if let Some(name) = fields.get("name").and_then(OtelModelValue::as_str) {
        span.set_attribute(KeyValue::new(
            format!("{prefix}.{index}.name"),
            name.to_owned(),
        ));
    }
    if let Some(description) = fields.get("description").and_then(OtelModelValue::as_str) {
        span.set_attribute(KeyValue::new(
            format!("{prefix}.{index}.description"),
            description.to_owned(),
        ));
    }
}

fn emit_optional_tool<S>(span: &mut S, index: usize, function: &MetadataMap)
where
    S: Span,
{
    emit_name_and_description(span, LLM_TOOLS, index, function);
    let declared = function
        .get("parameters")
        .filter(|value| !matches!(value, OtelModelValue::Null(())));
    if let Some(parameters) = declared
        && let Ok(text) = serde_json::to_string(parameters)
    {
        span.set_attribute(KeyValue::new(
            format!("{LLM_TOOLS}.{index}.parameters"),
            text,
        ));
    }
}

#[path = "arize_span_kind/infer_kind/tests.rs"]
mod infer_kind;
#[path = "arize_span_kind/tool_attrs/tests.rs"]
mod tool_attrs;
