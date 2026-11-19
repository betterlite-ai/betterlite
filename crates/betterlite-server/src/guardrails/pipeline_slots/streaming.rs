use std::collections::BTreeSet;

use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::GuardrailPipeline;

use super::slots::{policy_names, post_call_pipelines, step_guardrail_names};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingSupport {
    pub unified_execution: bool,
    pub streaming_execution: bool,
}

pub trait PipelineCallbacks {
    fn find_guardrail_callback(&self, guardrail_name: &str) -> Option<StreamingSupport>;
}

#[must_use]
pub fn step_supports_streaming<Translation>(
    guardrail_name: &str,
    translation: Option<&Translation>,
    callbacks: &dyn PipelineCallbacks,
) -> bool
where
    Translation: BaseTranslation,
{
    let Some(support) = callbacks.find_guardrail_callback(guardrail_name) else {
        return false;
    };
    if support.unified_execution {
        return true;
    }
    translation.is_some() && Translation::ASSEMBLES_STREAMED_RESPONSE && support.streaming_execution
}

#[must_use]
pub fn unsupported_stream_guardrails<Translation>(
    pipeline: &GuardrailPipeline,
    translation: Option<&Translation>,
    callbacks: &dyn PipelineCallbacks,
) -> Vec<String>
where
    Translation: BaseTranslation,
{
    let mut seen = BTreeSet::new();
    let mut unsupported = Vec::new();
    for step in &pipeline.steps {
        if step_supports_streaming(&step.guardrail, translation, callbacks) {
            continue;
        }
        if seen.insert(step.guardrail.clone()) {
            unsupported.push(step.guardrail.clone());
        }
    }
    unsupported
}

#[must_use]
pub fn pipeline_is_streamable<Translation>(
    policy_name: &str,
    pipeline: &GuardrailPipeline,
    translation: Option<&Translation>,
    callbacks: &dyn PipelineCallbacks,
) -> bool
where
    Translation: BaseTranslation,
{
    let unsupported = unsupported_stream_guardrails(pipeline, translation, callbacks);
    if unsupported.is_empty() {
        return true;
    }
    let guardrails = unsupported.join(", ");
    tracing::warn!(
        "Policy '{policy_name}' has post_call pipeline guardrails a streaming pipeline cannot run on this route yet; they need the unified apply_guardrail interface, or a post-call hook without a streaming iterator hook on a route whose translation assembles the streamed response. The stream skips the pipeline and its guardrails run on their own: {guardrails}"
    );
    false
}

#[must_use]
pub fn stream_gated_guardrail_names<Translation>(
    data: &JsonMap,
    translation: Option<&Translation>,
    callbacks: &dyn PipelineCallbacks,
) -> BTreeSet<String>
where
    Translation: BaseTranslation,
{
    let Some(found) = translation else {
        return BTreeSet::new();
    };
    let streamable: Vec<(String, GuardrailPipeline)> = post_call_pipelines(data)
        .into_iter()
        .filter(|(_, pipeline)| {
            unsupported_stream_guardrails(pipeline, Some(found), callbacks).is_empty()
        })
        .collect();
    step_guardrail_names(&streamable)
}

#[must_use]
pub fn streamable_post_call_pipelines<Translation>(
    data: &JsonMap,
    translation: Option<&Translation>,
    route: Option<&str>,
    callbacks: &dyn PipelineCallbacks,
) -> Vec<(String, GuardrailPipeline)>
where
    Translation: BaseTranslation,
{
    let post_call = post_call_pipelines(data);
    if post_call.is_empty() {
        return Vec::new();
    }
    let Some(found) = translation else {
        let names = policy_names(&post_call);
        let shown_route = route.unwrap_or_default();
        tracing::warn!(
            "Policies with post_call guardrail pipelines cannot scan streaming responses on route {shown_route} yet (no endpoint guardrail translation); the stream skips the pipelines and their guardrails run on their own: {names}"
        );
        return Vec::new();
    };
    post_call
        .into_iter()
        .filter(|(policy_name, pipeline)| {
            pipeline_is_streamable(policy_name, pipeline, Some(found), callbacks)
        })
        .collect()
}
