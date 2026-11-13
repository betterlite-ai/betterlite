use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_policy::attachment::PolicyAttachmentMatch;
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::inheritance::resolve_policy_guardrails;
use betterlite_policy::registry::{POLICY_VERSION_ID_PREFIX, PolicyRegistry};
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::metadata::{
    self, APPLIED_POLICIES, GUARDRAILS, POLICY_SOURCES,
};
use crate::api::request_guardrail_policy::{GuardrailPolicySettings, RequestGuardrailPolicyInput};

const MODEL: &str = "model";

// LiteLLM warns once per policy named in metadata that the registry does not hold; the rest still resolve
pub(super) fn add_policy_guardrails(
    input: &RequestGuardrailPolicyInput,
    registry: &PolicyRegistry,
    data: &mut RequestData,
    metadata_name: &str,
    settings: &GuardrailPolicySettings,
) -> Result<Vec<String>, RequestGuardrailPolicyError> {
    let policy_names = metadata_policy_names(input, settings)?;
    let mut excluded = Vec::new();
    if policy_names.is_empty() {
        return Ok(excluded);
    }
    tracing::debug!("Policy engine: resolving guardrails from key/team policies: {policy_names:?}");
    if !registry.initialized {
        tracing::debug!("Policy engine not initialized, skipping policy resolution from metadata");
        return Ok(excluded);
    }
    let context = PolicyMatchContext {
        team_alias: None,
        key_alias: None,
        model: request_model(data)?,
        tags: None,
    };
    let resolved = resolve_metadata_policies(registry, &policy_names, &context, &mut excluded);
    if resolved.is_empty() {
        return Ok(excluded);
    }
    write_policy_guardrails(data, metadata_name, &policy_names, resolved)?;
    Ok(excluded)
}

fn write_policy_guardrails(
    data: &mut RequestData,
    metadata_name: &str,
    policy_names: &[String],
    resolved: BTreeSet<String>,
) -> Result<(), RequestGuardrailPolicyError> {
    let bucket = metadata::bucket_mut(data, metadata_name)?;
    let mut combined = metadata::existing_string_list(bucket, GUARDRAILS)?;
    let mut present: BTreeSet<String> = combined.iter().cloned().collect();
    for guardrail in resolved {
        if present.insert(guardrail.clone()) {
            combined.push(guardrail);
        }
    }
    let mut applied = metadata::existing_string_list(bucket, APPLIED_POLICIES)?;
    applied.extend(policy_names.iter().cloned());
    bucket.insert(
        APPLIED_POLICIES.to_owned(),
        Json::List(applied.into_iter().map(Json::Text).collect()),
    );
    tracing::debug!(
        "Policy engine: added guardrails from key/team policies to request metadata: {combined:?}"
    );
    bucket.insert(
        GUARDRAILS.to_owned(),
        Json::List(combined.into_iter().map(Json::Text).collect()),
    );
    Ok(())
}

fn metadata_policy_names(
    input: &RequestGuardrailPolicyInput,
    settings: &GuardrailPolicySettings,
) -> Result<Vec<String>, RequestGuardrailPolicyError> {
    let mut policy_names = Vec::new();
    let mut seen = BTreeSet::new();
    for metadata in [
        &input.key_metadata,
        &input.team_metadata,
        &input.project_metadata,
    ] {
        if metadata.policies.is_empty() {
            continue;
        }
        metadata::require_premium(settings.premium_user)?;
        for policy_name in &metadata.policies {
            if seen.insert(policy_name.clone()) {
                policy_names.push(policy_name.clone());
            }
        }
    }
    Ok(policy_names)
}

fn resolve_metadata_policies(
    registry: &PolicyRegistry,
    policy_names: &[String],
    context: &PolicyMatchContext,
    excluded: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut resolved = BTreeSet::new();
    for policy_name in policy_names {
        if registry.policies.contains_key(policy_name) {
            let policy = resolve_policy_guardrails(policy_name, &registry.policies, Some(context));
            tracing::debug!(
                "Policy engine: resolved guardrails from policy '{policy_name}': {:?}",
                policy.guardrails
            );
            resolved.extend(policy.guardrails);
        } else {
            tracing::warn!("Policy engine: policy '{policy_name}' not found in registry");
            excluded.push(policy_name.clone());
        }
    }
    resolved
}

pub(super) fn request_model(
    data: &RequestData,
) -> Result<Option<String>, RequestGuardrailPolicyError> {
    match data.get(MODEL) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(model)) => Ok(Some(model.clone())),
        Some(_) => Err(RequestGuardrailPolicyError::NotTyped {
            field: MODEL,
            expected: "a string",
        }),
    }
}

pub(super) fn split_policy_names(policies: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut version_ids = Vec::new();
    for policy in policies {
        let Some(rest) = policy.strip_prefix(POLICY_VERSION_ID_PREFIX) else {
            names.push(policy);
            continue;
        };
        let policy_id = rest.trim();
        if !policy_id.is_empty() {
            version_ids.push(policy_id.to_owned());
        }
    }
    (names, version_ids)
}

pub(super) fn candidate_policies(
    matches: &[PolicyAttachmentMatch],
    body_policies: &[String],
) -> (Vec<String>, BTreeMap<String, String>) {
    let mut all_policies = Vec::new();
    let mut seen = BTreeSet::new();
    let mut reasons = BTreeMap::new();
    for policy_match in matches {
        if seen.insert(policy_match.policy_name.clone()) {
            all_policies.push(policy_match.policy_name.clone());
        }
        reasons.insert(
            policy_match.policy_name.clone(),
            policy_match.matched_via.clone(),
        );
    }
    tracing::debug!("Policy engine: matched policies via attachments: {all_policies:?}");
    for policy_name in body_policies {
        if seen.insert(policy_name.clone()) {
            all_policies.push(policy_name.clone());
        }
    }
    if !body_policies.is_empty() {
        tracing::debug!(
            "Policy engine: added dynamic policies from request body: {body_policies:?}"
        );
    }
    (all_policies, reasons)
}

pub(super) fn record_applied_policies(
    bucket: &mut IndexMap<String, Json>,
    applied: &[String],
    reasons: &BTreeMap<String, String>,
) -> Result<(), RequestGuardrailPolicyError> {
    let mut applied_metadata = metadata::existing_string_list(bucket, APPLIED_POLICIES)?;
    for policy_name in applied {
        if !applied_metadata.contains(policy_name) {
            applied_metadata.push(policy_name.clone());
        }
    }
    bucket.insert(
        APPLIED_POLICIES.to_owned(),
        Json::List(applied_metadata.into_iter().map(Json::Text).collect()),
    );
    let mut sources = metadata::policy_sources(bucket);
    for policy_name in applied {
        if let Some(reason) = reasons.get(policy_name) {
            sources.insert(policy_name.clone(), Json::Text(reason.clone()));
        }
    }
    if !sources.is_empty() {
        bucket.insert(POLICY_SOURCES.to_owned(), Json::Object(sources));
    }
    Ok(())
}
