mod input;
mod tests;
mod values;

use betterlite_logging::otel::integrations::arize_span_kind::{
    infer_open_inference_span_kind, set_tool_attributes,
};
use betterlite_logging::otel::integrations::langtrace::LangtraceAttributes;
use betterlite_logging::otel::mappers::base::{AttributeMapper as _, SpanData};
use betterlite_logging::otel::mappers::langfuse::LangfuseMapper;
use betterlite_logging::otel::mappers::langtrace::LangtraceMapper;
use betterlite_logging::otel::mappers::openinference::{OpenInferenceMapper, fit_indexed_messages};
use betterlite_logging::otel::mappers::registry::resolve_mappers;
use betterlite_logging::otel::mappers::weave::WeaveMapper;
use betterlite_logging::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use betterlite_logging::otel::model::trace_controls::TraceControls;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use input::{
    Input, IntegrationInput, LlmCallInput, RegistryInput, SheddingInput, ToolsInput, TraceInput,
};
use values::{attributes_record, events_record, helper_error, recorded_span};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::LangfuseLlmCall(call) => plain_record(call, Vocabulary::Langfuse)?,
            Input::LangtraceLlmCall(call) => plain_record(call, Vocabulary::Langtrace)?,
            Input::WeaveLlmCall(call) => plain_record(call, Vocabulary::Weave)?,
            Input::OpenInferenceLlmCall(call) => open_inference_record(call)?,
            Input::OpenInferenceMessageShedding(shedding) => shedding_record(shedding)?,
            Input::MapperRegistry(registry) => registry_record(registry)?,
            Input::LangtraceIntegration(integration) => integration_record(integration)?,
            Input::InferSpanKind(kind) => {
                json!({"kind": infer_open_inference_span_kind(kind.call_type.as_deref()).as_str()})
            }
            Input::ToolAttributes(tools) => tool_attributes_record(tools)?,
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

fn open_inference_record(call: &LlmCallInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(call)?;
    let mapper = OpenInferenceMapper {
        tool_attr_budget: call.tool_attr_budget,
    };
    let attributes = mapper.map(&SpanData::LLMCall(&span), &BTreeMap::new());
    Ok(attributes_record(&attributes))
}

fn shedding_record(shedding: &SheddingInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let span = llm_call_span(&shedding.call)?;
    let mapper = OpenInferenceMapper {
        tool_attr_budget: shedding.call.tool_attr_budget,
    };
    let attributes = mapper.map(&SpanData::LLMCall(&span), &BTreeMap::new());
    Ok(attributes_record(&fit_indexed_messages(
        attributes,
        shedding.budget,
    )))
}

fn registry_record(registry: &RegistryInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mappers = resolve_mappers(&registry.names)
        .map_err(|error| helper_error("MapperError", error.to_string()))?;
    let entries: Vec<Value> = mappers
        .iter()
        .map(|mapper| {
            json!({"mapper": mapper.name(), "tool_attr_budget": mapper.tool_attr_budget()})
        })
        .collect();
    Ok(Value::Array(entries))
}

fn integration_record(
    integration: &IntegrationInput,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let (attributes, events) = recorded_span(|span| {
        LangtraceAttributes::set_langtrace_attributes(
            span,
            &integration.kwargs,
            &integration.response,
        );
    })?;
    Ok(json!({"attributes": attributes_record(&attributes), "events": events_record(&events)}))
}

fn tool_attributes_record(tools: &ToolsInput) -> Result<Value, Box<crate::helpers::HelperError>> {
    let (attributes, _events) = recorded_span(|span| {
        set_tool_attributes(
            span,
            tools.optional_tools.as_deref(),
            tools.metadata_tools.as_deref(),
        );
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
