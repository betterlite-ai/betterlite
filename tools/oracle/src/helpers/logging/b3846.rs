mod input;
mod tests;
mod values;

use betterlite_logging::otel::integrations::arize_span_kind::{
    infer_open_inference_span_kind, set_tool_attributes,
};
use betterlite_logging::otel::integrations::langtrace::LangtraceAttributes;
use betterlite_logging::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use betterlite_logging::otel::mappers::langfuse::LangfuseMapper;
use betterlite_logging::otel::mappers::langtrace::LangtraceMapper;
use betterlite_logging::otel::mappers::openinference::{OpenInferenceMapper, fit_indexed_messages};
use betterlite_logging::otel::mappers::registry::resolve_mappers;
use betterlite_logging::otel::mappers::weave::WeaveMapper;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use betterlite_logging::otel::model::trace_controls::TraceControls;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use input::{Input, LlmCallInput, TraceInput};
use values::{attributes_record, helper_error, recorded_span};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::LangfuseLlmCall(call) => plain_record(call, Vocabulary::Langfuse)?,
            Input::LangtraceLlmCall(call) => plain_record(call, Vocabulary::Langtrace)?,
            Input::WeaveLlmCall(call) => plain_record(call, Vocabulary::Weave)?,
            Input::OpenInferenceLlmCall(call) => {
                attributes_record(&open_inference_attributes(call)?)
            }
            Input::OpenInferenceMessageShedding(shedding) => {
                let attributes = open_inference_attributes(&shedding.call)?;
                attributes_record(&fit_indexed_messages(attributes, shedding.budget))
            }
            Input::MapperRegistry { names } => registry_record(names)?,
            Input::LangtraceIntegration { kwargs, response } => {
                integration_record(kwargs, response)?
            }
            Input::InferSpanKind { call_type } => {
                json!({"kind": infer_open_inference_span_kind(call_type.as_deref()).to_string()})
            }
            Input::ToolAttributes {
                optional_tools,
                metadata_tools,
            } => tool_attributes_record(optional_tools.as_deref(), metadata_tools.as_deref())?,
        };
        Ok(json!({"result": value}))
    }
}

#[derive(Clone, Copy)]
enum Vocabulary {
    Langfuse,
    Langtrace,
    Weave,
}

fn plain_record(
    call: &LlmCallInput,
    vocabulary: Vocabulary,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(call)?;
    let data = SpanData::LLMCall(&span);
    let mapped = match vocabulary {
        Vocabulary::Langfuse => LangfuseMapper.map(&data, &BTreeMap::new()),
        Vocabulary::Langtrace => LangtraceMapper.map(&data, &BTreeMap::new()),
        Vocabulary::Weave => WeaveMapper.map(&data, &BTreeMap::new()),
    };
    Ok(attributes_record(&mapped))
}

fn open_inference_attributes(
    call: &LlmCallInput,
) -> Result<AttributeMap, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(call)?;
    let mapper = OpenInferenceMapper {
        tool_attr_budget: call.tool_attr_budget,
    };
    Ok(mapper.map(&SpanData::LLMCall(&span), &BTreeMap::new()))
}

fn registry_record(names: &[String]) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mappers =
        resolve_mappers(names).map_err(|error| helper_error("MapperError", error.to_string()))?;
    let entries: Vec<Value> = mappers
        .iter()
        .map(|mapper| {
            json!({"mapper": mapper.name(), "tool_attr_budget": mapper.tool_attr_budget()})
        })
        .collect();
    Ok(Value::Array(entries))
}

fn integration_record(
    kwargs: &MetadataMap,
    response: &MetadataMap,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let (attributes, events) = recorded_span(|span| {
        LangtraceAttributes::set_langtrace_attributes(span, kwargs, response);
    })?;
    let recorded: Vec<Value> = events
        .iter()
        .map(|(name, fields)| json!({"name": name, "attributes": attributes_record(fields)}))
        .collect();
    Ok(json!({"attributes": attributes_record(&attributes), "events": recorded}))
}

fn tool_attributes_record(
    optional_tools: Option<&[OtelModelValue]>,
    metadata_tools: Option<&[OtelModelValue]>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let (attributes, _events) = recorded_span(|span| {
        set_tool_attributes(span, optional_tools, metadata_tools);
    })?;
    Ok(attributes_record(&attributes))
}

fn llm_call_span(call: &LlmCallInput) -> Result<LLMCallSpanData, Box<crate::helpers::HelperError>> {
    LLMCallSpanData::from_standard_logging_payload(
        &call.payload,
        LLMCallOptions {
            capture_content: call.capture_content,
            time_to_first_chunk_seconds: call.time_to_first_chunk_seconds,
            request_route: call.request_route.clone(),
            trace: call
                .trace
                .as_ref()
                .map_or_else(TraceControls::default, trace_controls),
            session_id: call.session_id.clone(),
        },
    )
    .map_err(|error| helper_error("MetadataError", error.to_string()))
}

fn trace_controls(trace: &TraceInput) -> TraceControls {
    TraceControls {
        name: trace.name.clone(),
        user_id: trace.user_id.clone(),
        session_id: trace.session_id.clone(),
        tags: trace.tags.clone(),
    }
}
