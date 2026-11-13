use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::matcher::{policies_matching_conditions, policy_applies};
use betterlite_policy::registry::PolicyRegistry;
use betterlite_policy::resolver::guardrails::resolve_guardrails_for_context;
use betterlite_policy::resolver::pipelines::resolve_pipelines_for_context;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use crate::api::request_destination::RequestData;
use crate::api::request_destination::parameters::tags_from_request_body;
use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::metadata::{self, GUARDRAILS};
use crate::api::request_guardrail_policy::pipelines::write_pipelines;
use crate::api::request_guardrail_policy::policies::{
    candidate_policies, record_applied_policies, request_model, split_policy_names,
};
use crate::api::request_guardrail_policy::{GuardrailPolicyService, RequestGuardrailPolicyInput};

const REQUEST_POLICIES: &str = "policies";

pub(super) fn add_policy_engine_guardrails(
    service: &GuardrailPolicyService<'_>,
    input: &RequestGuardrailPolicyInput,
    data: &mut RequestData,
) -> Result<(), RequestGuardrailPolicyError> {
    let request_policies = take_request_policies(data, service.settings.limits)?;
    let registry = service.policies;
    tracing::debug!(
        "Policy engine: registry initialized={}, policy_count={}",
        registry.initialized,
        registry.policies.len()
    );
    if !registry.initialized {
        tracing::debug!("Policy engine not initialized, skipping policy matching");
        return Ok(());
    }
    let context = engine_context(input, data)?;
    tracing::debug!(
        "Policy engine: matching policies for context team_alias={:?}, key_alias={:?}, model={:?}, tags={:?}",
        context.team_alias,
        context.key_alias,
        context.model,
        context.tags
    );
    let (names, version_ids) = split_policy_names(request_policies);
    let version_view = version_policy_view(registry, &version_ids);
    let mut body_policies = names;
    if let Some(view) = &version_view {
        body_policies.extend(view.fetched.iter().cloned());
    }
    let active = version_view
        .as_ref()
        .map_or(registry, |view| &view.registry);
    let mut engine = Engine {
        service,
        data,
        metadata_name: &input.metadata_variable_name,
    };
    let applied = engine.match_and_track(&context, &body_policies, active)?;
    engine.apply_resolved(&context, &applied, active)
}

fn engine_context(
    input: &RequestGuardrailPolicyInput,
    data: &RequestData,
) -> Result<PolicyMatchContext, RequestGuardrailPolicyError> {
    let tags = tags_from_request_body(data);
    Ok(PolicyMatchContext {
        team_alias: input.principal.team_alias.clone(),
        key_alias: input.principal.key_alias.clone(),
        model: request_model(data)?,
        tags: (!tags.is_empty()).then_some(tags),
    })
}

fn take_request_policies(
    data: &mut RequestData,
    limits: ApiLimits,
) -> Result<Vec<String>, RequestGuardrailPolicyError> {
    let Some(value) = data.shift_remove(REQUEST_POLICIES) else {
        return Ok(Vec::new());
    };
    match value {
        Json::Null => Ok(Vec::new()),
        Json::List(items) => {
            limits.check(ApiBoundariesInput::Items(ItemsBoundary {
                known_count: items.len(),
            }))?;
            metadata::texts(items, REQUEST_POLICIES)
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(RequestGuardrailPolicyError::NotTyped {
                field: REQUEST_POLICIES,
                expected: "a list of strings",
            })
        }
    }
}

struct VersionPolicies {
    registry: PolicyRegistry,
    fetched: Vec<String>,
}

fn version_policy_view(
    registry: &PolicyRegistry,
    version_ids: &[String],
) -> Option<VersionPolicies> {
    if version_ids.is_empty() {
        return None;
    }
    let mut policies = registry.policies.clone();
    let mut fetched = Vec::new();
    for policy_id in version_ids {
        if let Some((name, policy)) = registry.policies_by_id.get(policy_id) {
            policies.insert(name.clone(), policy.clone());
            fetched.push(name.clone());
            tracing::debug!("Policy engine: loaded version by ID policy_{policy_id} -> {name}");
        } else {
            tracing::debug!(
                "Policy engine: policy version {policy_id} not found in cache, skipping"
            );
        }
    }
    Some(VersionPolicies {
        registry: PolicyRegistry {
            policies,
            config_policies: BTreeMap::new(),
            sources: BTreeMap::new(),
            policies_by_id: BTreeMap::new(),
            initialized: true,
        },
        fetched,
    })
}

struct Engine<'request, 'repositories> {
    service: &'request GuardrailPolicyService<'repositories>,
    data: &'request mut RequestData,
    metadata_name: &'request str,
}

impl Engine<'_, '_> {
    fn match_and_track(
        &mut self,
        context: &PolicyMatchContext,
        body_policies: &[String],
        registry: &PolicyRegistry,
    ) -> Result<Vec<String>, RequestGuardrailPolicyError> {
        let matches = self.service.attachments.attached_policy_matches(
            context,
            Some(&|policy_name: &str| policy_applies(policy_name, context, &registry.policies)),
        );
        let (all_policies, reasons) = candidate_policies(&matches, body_policies);
        if all_policies.is_empty() {
            return Ok(Vec::new());
        }
        let applied = policies_matching_conditions(&all_policies, context, &registry.policies);
        tracing::debug!("Policy engine: applied policies (conditions matched): {applied:?}");
        if applied.is_empty() {
            return Ok(Vec::new());
        }
        let bucket = metadata::bucket_mut(self.data, self.metadata_name)?;
        record_applied_policies(bucket, &applied, &reasons)?;
        Ok(applied)
    }

    fn apply_resolved(
        &mut self,
        context: &PolicyMatchContext,
        applied: &[String],
        registry: &PolicyRegistry,
    ) -> Result<(), RequestGuardrailPolicyError> {
        let policy_names = (!applied.is_empty()).then_some(applied);
        let guardrails = resolve_guardrails_for_context(
            context,
            registry,
            self.service.attachments,
            policy_names,
        );
        tracing::debug!("Policy engine: resolved guardrails: {guardrails:?}");
        let pipelines = resolve_pipelines_for_context(
            context,
            registry,
            self.service.attachments,
            policy_names,
        );
        let bucket = metadata::bucket_mut(self.data, self.metadata_name)?;
        write_pipelines(bucket, &pipelines)?;
        if guardrails.is_empty() && pipelines.is_empty() {
            return Ok(());
        }
        write_resolved_guardrails(bucket, guardrails)
    }
}

fn write_resolved_guardrails(
    bucket: &mut IndexMap<String, Json>,
    guardrails: Vec<String>,
) -> Result<(), RequestGuardrailPolicyError> {
    let mut combined = metadata::existing_string_list(bucket, GUARDRAILS)?;
    let mut present: BTreeSet<String> = combined.iter().cloned().collect();
    for guardrail in guardrails {
        if present.insert(guardrail.clone()) {
            combined.push(guardrail);
        }
    }
    tracing::debug!("Policy engine: added guardrails to request metadata: {combined:?}");
    bucket.insert(
        GUARDRAILS.to_owned(),
        Json::List(combined.into_iter().map(Json::Text).collect()),
    );
    Ok(())
}
