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
    let (tag_matched, body_selected) = deferral_matches(data, policies);
    if !tag_matched.is_empty() {
        let tag_names = tag_matched.join(", ");
        tracing::warn!(
            "Policy engine: background response {response_id} matched post_call policies through a request tag at submit; retrieval re-matches only the key, team, and model scopes, so a tag carried in the request body does not govern the completed response: {tag_names}"
        );
    }
    if !body_selected.is_empty() {
        let body_names = body_selected.join(", ");
        tracing::warn!(
            "Policy engine: background response {response_id} matched post_call policies through the request body's policies list at submit; retrieval carries no request body, so those policies do not govern the completed response: {body_names}"
        );
    }
    withdraw_deferred_claims(data, policies, context);
}

fn deferral_matches(
    data: &JsonMap,
    policies: &[(String, GuardrailPipeline)],
) -> (Vec<String>, Vec<String>) {
    let sources = policy_sources(data);
    let mut tag_matched = Vec::new();
    let mut body_selected = Vec::new();
    for (policy_name, _) in policies {
        let attributed = sources.and_then(|bucket| bucket.get(policy_name));
        let matched_by_tag = attributed
            .and_then(JsonValue::as_str)
            .is_some_and(|reason| reason.contains(TAG_MARK));
        if matched_by_tag {
            tag_matched.push(policy_name.clone());
        }
        if attributed.is_none() {
            body_selected.push(policy_name.clone());
        }
    }
    (tag_matched, body_selected)
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
    without_names(
        bucket,
        &[
            (APPLIED_POLICIES, &withdrawn_policies),
            (APPLIED_GUARDRAILS, &withdrawn_guardrails),
        ],
    );
    let JsonValue::Object(state) = bucket else {
        return;
    };
    let Some(JsonValue::Object(sources)) = state.get_mut(POLICY_SOURCES) else {
        return;
    };
    sources.retain(|policy_name, _| !withdrawn_policies.contains(policy_name));
    if sources.is_empty() {
        state.remove(POLICY_SOURCES);
    }
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

fn without_names(bucket_value: &mut JsonValue, slots: &[(&str, &BTreeSet<String>)]) {
    let JsonValue::Object(bucket) = bucket_value else {
        return;
    };
    for &(slot, names) in slots {
        let Some(JsonValue::Array(claimed)) = bucket.get_mut(slot) else {
            continue;
        };
        claimed.retain(|name| name.as_str().is_none_or(|text| !names.contains(text)));
        if claimed.is_empty() {
            bucket.remove(slot);
        }
    }
}
