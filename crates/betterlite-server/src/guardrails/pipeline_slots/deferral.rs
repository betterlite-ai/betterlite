use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineMode};

use super::slots::{
    pipeline_managed_names, policy_names, policy_state_metadata, step_guardrail_names,
};

const METADATA: &str = "metadata";
const LITELLM_METADATA: &str = "litellm_metadata";
const APPLIED_GUARDRAILS: &str = "applied_guardrails";
const APPLIED_POLICIES: &str = "applied_policies";
const POLICY_SOURCES: &str = "policy_sources";
const TAG_MARK: &str = "tag:";

pub trait DeferralContext {
    fn resolved_policy_guardrails(&self, policy_name: &str) -> Vec<String>;
    fn standalone_pre_call_guardrails(&self, data: &JsonMap) -> BTreeSet<String>;
}

pub fn defer_post_call_pipelines(
    data: &mut JsonMap,
    response_id: &str,
    status: &str,
    policies: &[(String, GuardrailPipeline)],
    context: &dyn DeferralContext,
) {
    if policies.is_empty() {
        return;
    }
    let names = policy_names(policies);
    tracing::debug!(
        "Post_call guardrail pipelines wait for background response {response_id} (status={status}) to be retrieved complete: {names}"
    );
    let tag_matched = tag_matched_deferrals(data, policies);
    if !tag_matched.is_empty() {
        let tag_names = tag_matched.join(", ");
        tracing::warn!(
            "Policy engine: background response {response_id} matched post_call policies through a request tag at submit; retrieval re-matches only the key, team, and model scopes, so a tag carried in the request body does not govern the completed response: {tag_names}"
        );
    }
    let body_selected = body_selected_deferrals(data, policies);
    if !body_selected.is_empty() {
        let body_names = body_selected.join(", ");
        tracing::warn!(
            "Policy engine: background response {response_id} matched post_call policies through the request body's policies list at submit; retrieval carries no request body, so those policies do not govern the completed response: {body_names}"
        );
    }
    withdraw_deferred_claims(data, policies, context);
}

fn tag_matched_deferrals(data: &JsonMap, policies: &[(String, GuardrailPipeline)]) -> Vec<String> {
    let Some(sources) = policy_sources(data) else {
        return Vec::new();
    };
    policies
        .iter()
        .filter(|(policy_name, _)| {
            sources
                .get(policy_name)
                .and_then(JsonValue::as_str)
                .is_some_and(|reason| reason.contains(TAG_MARK))
        })
        .map(|(policy_name, _)| policy_name.clone())
        .collect()
}

fn body_selected_deferrals(
    data: &JsonMap,
    policies: &[(String, GuardrailPipeline)],
) -> Vec<String> {
    let attributed = policy_sources(data);
    policies
        .iter()
        .filter(|(policy_name, _)| {
            attributed.is_none_or(|sources| !sources.contains_key(policy_name))
        })
        .map(|(policy_name, _)| policy_name.clone())
        .collect()
}

fn policy_sources(data: &JsonMap) -> Option<&JsonMap> {
    policy_state_metadata(data)?
        .get(POLICY_SOURCES)?
        .as_object()
}

fn withdraw_deferred_claims(
    data: &mut JsonMap,
    deferred: &[(String, GuardrailPipeline)],
    context: &dyn DeferralContext,
) {
    let (outside_by_policy, running_elsewhere) = outside_sets(data, deferred, context);
    let withdrawn_policies: BTreeSet<String> = outside_by_policy
        .iter()
        .filter(|(_, outside)| outside.is_empty())
        .map(|(policy_name, _)| policy_name.clone())
        .collect();
    let step_names = step_guardrail_names(deferred);
    let withdrawn_guardrails: BTreeSet<String> =
        step_names.difference(&running_elsewhere).cloned().collect();
    let bucket = metadata_bucket_mut(data);
    without_names(bucket, APPLIED_POLICIES, &withdrawn_policies);
    without_names(bucket, APPLIED_GUARDRAILS, &withdrawn_guardrails);
    drop_withdrawn_sources(bucket, &withdrawn_policies);
}

fn outside_sets(
    data: &JsonMap,
    deferred: &[(String, GuardrailPipeline)],
    context: &dyn DeferralContext,
) -> (Vec<(String, BTreeSet<String>)>, BTreeSet<String>) {
    let mut running = pipeline_managed_names(data, PipelineMode::PreCall);
    running.extend(context.standalone_pre_call_guardrails(data));
    let mut outside_by_policy: Vec<(String, BTreeSet<String>)> = Vec::new();
    for (policy_name, pipeline) in deferred {
        let resolved: BTreeSet<String> = context
            .resolved_policy_guardrails(policy_name)
            .into_iter()
            .collect();
        let steps: BTreeSet<String> = pipeline
            .steps
            .iter()
            .map(|step| step.guardrail.clone())
            .collect();
        let outside: BTreeSet<String> = resolved.difference(&steps).cloned().collect();
        running.extend(outside.iter().cloned());
        outside_by_policy.push((policy_name.clone(), outside));
    }
    (outside_by_policy, running)
}

fn metadata_bucket_mut(data: &mut JsonMap) -> &mut JsonValue {
    let bucket_key = if data.contains_key(LITELLM_METADATA) {
        LITELLM_METADATA
    } else {
        METADATA
    };
    let value = data
        .entry(bucket_key.to_owned())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    if !value.is_object() {
        *value = JsonValue::Object(JsonMap::new());
    }
    value
}

fn without_names(bucket_value: &mut JsonValue, slot: &str, names: &BTreeSet<String>) {
    let JsonValue::Object(bucket) = bucket_value else {
        return;
    };
    let Some(JsonValue::Array(claimed)) = bucket.get(slot) else {
        return;
    };
    let remaining: Vec<JsonValue> = claimed
        .iter()
        .filter(|name| name.as_str().is_none_or(|text| !names.contains(text)))
        .cloned()
        .collect();
    if remaining.is_empty() {
        bucket.remove(slot);
    } else {
        bucket.insert(slot.to_owned(), JsonValue::Array(remaining));
    }
}

fn drop_withdrawn_sources(bucket_value: &mut JsonValue, withdrawn: &BTreeSet<String>) {
    let JsonValue::Object(bucket) = bucket_value else {
        return;
    };
    let Some(JsonValue::Object(sources)) = bucket.get(POLICY_SOURCES) else {
        return;
    };
    let remaining: JsonMap = sources
        .iter()
        .filter(|(policy_name, _)| !withdrawn.contains(*policy_name))
        .map(|(policy_name, reason)| (policy_name.clone(), reason.clone()))
        .collect();
    if remaining.is_empty() {
        bucket.remove(POLICY_SOURCES);
    } else {
        bucket.insert(POLICY_SOURCES.to_owned(), JsonValue::Object(remaining));
    }
}
