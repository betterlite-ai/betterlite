use betterlite_models::decisions::Json;
use opentelemetry::KeyValue;
use opentelemetry::trace::{TraceId, Tracer};

use crate::langfuse_child_observations::child_spans::ChildSpans;
use crate::langfuse_child_observations::grounding::log_grounding_spans;
use crate::langfuse_child_observations::guardrails::log_guardrail_spans;
use crate::langfuse_child_observations::prompts::GenerationPrompt;
use crate::langfuse_observations::attributes::{
    ObservationAttributes, ObservationType, PromptLink, TraceAttributes, observation_attributes,
    trace_attributes,
};
use crate::langfuse_observations::identity::resolve_observation_id;
use crate::langfuse_observations::span::{GenerationStart, start_generation};
use crate::langfuse_projections::steering::trace_public_flag;
use crate::langfuse_projections::text::optional_str;
use crate::langfuse_rest::records::Prompt;

use super::generation::GenerationParams;
use super::trace_ids::CallTrace;
use super::trace_params::TraceParams;
use super::{AssemblyRuntime, GenerationTrace};

pub struct ObservationRequest<'request, T>
where
    T: Tracer,
{
    pub trace: &'request GenerationTrace<'request>,
    pub call: &'request CallTrace,
    pub params: &'request GenerationParams,
    pub plan: &'request TraceParams,
    pub runtime: &'request AssemblyRuntime<'request, T>,
    pub trace_id: TraceId,
}

// _log_langfuse_v2 from start_generation to the generation's end, child observations included
#[must_use]
pub fn observe<T>(request: &ObservationRequest<'_, T>) -> String
where
    T: Tracer,
{
    let params = request.params;
    let public = trace_public_flag(request.plan.params.get("public"));
    let parent = optional_str(request.trace.metadata.get("parent_observation_id"));
    let mut observation = start_generation(
        request.runtime.tracer,
        request.runtime.id_generator,
        GenerationStart {
            trace_id: request.trace_id,
            parent_observation_id: resolve_observation_id(parent.as_deref()),
            existing_trace: request.call.existing.is_some(),
            observation_id: resolve_observation_id(params.id.as_deref()),
            name: params.name.clone(),
            start_time: request.trace.start_time,
            public,
            attributes: attributes(request),
        },
    );
    let spans = ChildSpans {
        tracer: request.runtime.tracer,
        parent: &observation,
    };
    log_grounding_spans(&spans, &params.enrichments);
    log_guardrail_spans(&spans, request.trace.payload);
    observation.end(request.trace.end_time);
    observation.id.clone()
}

fn attributes<T>(request: &ObservationRequest<'_, T>) -> Vec<KeyValue>
where
    T: Tracer,
{
    let params = request.params;
    let trace_root = request.call.existing.is_none()
        && optional_str(request.trace.metadata.get("parent_observation_id")).is_none();
    let level = Json::Text(request.trace.level.to_owned());
    let model = Json::Text(params.model.clone());
    let model_parameters = Json::Object(params.model_parameters.clone());
    let status_message = params.status_message.clone().map(Json::Text);
    let version = params.version.clone().map(Json::Text);
    let prompt = prompt_link(params.prompt.as_ref());
    let observation = observation_attributes(&ObservationAttributes {
        observation_type: ObservationType::Generation,
        input: params.input.as_ref(),
        output: params.output.as_ref(),
        metadata: Some(&params.metadata),
        level: Some(&level),
        status_message: status_message.as_ref(),
        version: version.as_ref(),
        model: Some(&model),
        model_parameters: Some(&model_parameters),
        usage_details: params.usage_details.as_ref(),
        cost_details: params.cost_details.as_ref(),
        completion_start_time: request.trace.completion_start_time,
        prompt: prompt.as_ref(),
    });
    merge(observation, trace_level_attributes(request, trace_root))
}

fn trace_level_attributes<T>(request: &ObservationRequest<'_, T>, trace_root: bool) -> Vec<KeyValue>
where
    T: Tracer,
{
    let planned = &request.plan.params;
    let trace_input = trace_io(
        planned.get("input"),
        request.params.input.as_ref(),
        trace_root,
    );
    let trace_output = trace_io(
        planned.get("output"),
        request.params.output.as_ref(),
        trace_root,
    );
    trace_attributes(&TraceAttributes {
        name: planned.get("name"),
        user_id: planned.get("user_id"),
        session_id: planned.get("session_id"),
        version: planned.get("version"),
        release: planned.get("release"),
        tags: planned.get("tags"),
        metadata: planned.get("metadata"),
        public: trace_public_flag(planned.get("public")),
        input: trace_input.as_ref(),
        output: trace_output.as_ref(),
    })
}

// on the root observation the server takes the generation's own input and output for the trace's
fn trace_io(
    trace_value: Option<&Json>,
    generation_value: Option<&Json>,
    trace_root: bool,
) -> Option<Json> {
    let value = trace_value?;
    if matches!(value, Json::Null) {
        return None;
    }
    if trace_root && generation_value == Some(value) {
        return None;
    }
    Some(value.clone())
}

// the trace attributes win over the generation's on a shared key, as the source's dict merge does
fn merge(observation: Vec<KeyValue>, trace: Vec<KeyValue>) -> Vec<KeyValue> {
    let trace_keys: Vec<&str> = trace.iter().map(|pair| pair.key.as_str()).collect();
    let mut merged: Vec<KeyValue> = observation
        .into_iter()
        .filter(|pair| !trace_keys.contains(&pair.key.as_str()))
        .collect();
    merged.extend(trace);
    merged
}

fn prompt_link(prompt: Option<&GenerationPrompt>) -> Option<PromptLink> {
    let Some(GenerationPrompt::Client(linked)) = prompt else {
        return None;
    };
    let meta = match linked.as_ref() {
        Prompt::Chat(chat) => &chat.meta,
        Prompt::Text(text) => &text.meta,
    };
    Some(PromptLink {
        name: meta.name.clone(),
        version: meta.version,
        is_fallback: false,
    })
}
