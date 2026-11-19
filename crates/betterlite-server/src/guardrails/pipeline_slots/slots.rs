use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineMode};

const METADATA: &str = "metadata";
const LITELLM_METADATA: &str = "litellm_metadata";
const GUARDRAIL_PIPELINES: &str = "_guardrail_pipelines";
const PIPELINE_MANAGED_GUARDRAILS: &str = "_pipeline_managed_guardrails";
const GUARDRAILS: &str = "guardrails";

#[must_use]
pub fn policy_state_metadata(data: &JsonMap) -> Option<&JsonMap> {
    [data.get(METADATA), data.get(LITELLM_METADATA)]
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
        .find(|bucket| {
            bucket.contains_key(GUARDRAIL_PIPELINES)
                || bucket.contains_key(PIPELINE_MANAGED_GUARDRAILS)
        })
}

#[must_use]
pub fn policy_pipelines(data: &JsonMap) -> Vec<(String, GuardrailPipeline)> {
    let Some(bucket) = policy_state_metadata(data) else {
        return Vec::new();
    };
    let Some(JsonValue::Array(entries)) = bucket.get(GUARDRAIL_PIPELINES) else {
        return Vec::new();
    };
    let mut pipelines = Vec::with_capacity(entries.len());
    for entry in entries {
        match pipeline_pair(entry) {
            Some(pair) => pipelines.push(pair),
            None => tracing::warn!(
                "Policy engine: ignoring a stored guardrail pipeline that is not a [policy name, pipeline] pair"
            ),
        }
    }
    pipelines
}

fn pipeline_pair(entry: &JsonValue) -> Option<(String, GuardrailPipeline)> {
    let JsonValue::Array(parts) = entry else {
        return None;
    };
    let [JsonValue::String(policy_name), pipeline] = parts.as_slice() else {
        return None;
    };
    let parsed = serde_json::from_value(pipeline.clone()).ok()?;
    Some((policy_name.clone(), parsed))
}

#[must_use]
pub fn step_guardrail_names(pipelines: &[(String, GuardrailPipeline)]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (_, pipeline) in pipelines {
        names.extend(pipeline.steps.iter().map(|step| step.guardrail.clone()));
    }
    names
}

#[must_use]
pub fn pipeline_managed_names(data: &JsonMap, mode: PipelineMode) -> BTreeSet<String> {
    let matching: Vec<(String, GuardrailPipeline)> = policy_pipelines(data)
        .into_iter()
        .filter(|(_, pipeline)| pipeline.mode == mode)
        .collect();
    step_guardrail_names(&matching)
}

#[must_use]
pub fn post_call_pipelines(data: &JsonMap) -> Vec<(String, GuardrailPipeline)> {
    policy_pipelines(data)
        .into_iter()
        .filter(|(_, pipeline)| pipeline.mode == PipelineMode::PostCall)
        .collect()
}

pub(super) fn policy_names(pipelines: &[(String, GuardrailPipeline)]) -> String {
    pipelines
        .iter()
        .map(|(policy_name, _)| policy_name.as_str())
        .collect::<Vec<&str>>()
        .join(", ")
}

pub fn merge_pipeline_metadata_writes(data: &mut JsonMap, modified: &JsonMap) {
    for bucket_key in [METADATA, LITELLM_METADATA] {
        merge_pipeline_metadata_bucket(data, bucket_key, modified.get(bucket_key));
    }
}

fn merge_pipeline_metadata_bucket(
    data: &mut JsonMap,
    bucket_key: &str,
    modified_bucket: Option<&JsonValue>,
) {
    let Some(JsonValue::Object(writes)) = modified_bucket else {
        return;
    };
    let surviving: JsonMap = writes
        .iter()
        .filter(|(key, _)| key.as_str() != GUARDRAILS)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    match data.get_mut(bucket_key) {
        Some(JsonValue::Object(existing)) => {
            existing.extend(surviving);
        }
        Some(existing) => {
            *existing = JsonValue::Object(surviving);
        }
        None => {
            data.insert(bucket_key.to_owned(), JsonValue::Object(surviving));
        }
    }
}

#[must_use]
pub fn is_pending_background_response(response: &ResponsesApiResponse) -> bool {
    let Some(status) = response.status.as_deref() else {
        return false;
    };
    matches!(status, "queued" | "in_progress")
}
