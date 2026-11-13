use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_policy::attachment::PolicyAttachmentMatch;
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::inheritance::resolve_policy_guardrails;
use betterlite_policy::matcher::{policies_matching_conditions, policy_applies};
use betterlite_policy::pipeline::{GuardrailPipeline, PipelineAction, PipelineMode};
use betterlite_policy::registry::{POLICY_VERSION_ID_PREFIX, PolicyRegistry};
use betterlite_policy::resolver::guardrails::resolve_guardrails_for_context;
use betterlite_policy::resolver::pipelines::{
    pipeline_managed_guardrails, resolve_pipelines_for_context,
};
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use crate::api::request_destination::RequestData;
use crate::api::request_destination::parameters::tags_from_request_body;
use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::metadata::{
    self, APPLIED_POLICIES, GUARDRAILS, POLICY_SOURCES,
};
use crate::api::request_guardrail_policy::{
    GuardrailPolicyService, GuardrailPolicySettings, RequestGuardrailPolicyInput,
};

const REQUEST_POLICIES: &str = "policies";
const MODEL: &str = "model";
const GUARDRAIL_PIPELINES: &str = "_guardrail_pipelines";
const MANAGED_GUARDRAILS: &str = "_pipeline_managed_guardrails";

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

fn split_policy_names(policies: Vec<String>) -> (Vec<String>, Vec<String>) {
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
        write_pipelines(bucket, &pipelines);
        if guardrails.is_empty() && pipelines.is_empty() {
            return Ok(());
        }
        write_resolved_guardrails(bucket, guardrails)
    }
}

fn write_pipelines(bucket: &mut IndexMap<String, Json>, pipelines: &[(String, GuardrailPipeline)]) {
    if pipelines.is_empty() {
        return;
    }
    let managed = pipeline_managed_guardrails(pipelines);
    tracing::debug!(
        "Policy engine: resolved {} pipeline(s), managed guardrails: {:?}",
        pipelines.len(),
        managed
    );
    bucket.insert(GUARDRAIL_PIPELINES.to_owned(), pipelines_json(pipelines));
    bucket.insert(
        MANAGED_GUARDRAILS.to_owned(),
        Json::List(managed.into_iter().map(Json::Text).collect()),
    );
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

fn candidate_policies(
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

fn record_applied_policies(
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

fn pipelines_json(pipelines: &[(String, GuardrailPipeline)]) -> Json {
    let mut entries = Vec::with_capacity(pipelines.len());
    for (policy_name, pipeline) in pipelines {
        entries.push(Json::List(vec![
            Json::Text(policy_name.clone()),
            pipeline_json(pipeline),
        ]));
    }
    Json::List(entries)
}

fn pipeline_json(pipeline: &GuardrailPipeline) -> Json {
    let mut steps = Vec::with_capacity(pipeline.steps.len());
    for step in &pipeline.steps {
        let mut fields = IndexMap::new();
        fields.insert("guardrail".to_owned(), Json::Text(step.guardrail.clone()));
        fields.insert(
            "on_fail".to_owned(),
            Json::Text(pipeline_action_name(step.on_fail).to_owned()),
        );
        fields.insert(
            "on_pass".to_owned(),
            Json::Text(pipeline_action_name(step.on_pass).to_owned()),
        );
        fields.insert(
            "on_error".to_owned(),
            step.on_error.map_or(Json::Null, |action| {
                Json::Text(pipeline_action_name(action).to_owned())
            }),
        );
        fields.insert("pass_data".to_owned(), Json::Bool(step.pass_data));
        fields.insert(
            "modify_response_message".to_owned(),
            step.modify_response_message
                .as_ref()
                .map_or(Json::Null, |message| Json::Text(message.clone())),
        );
        steps.push(Json::Object(fields));
    }
    let mut fields = IndexMap::new();
    fields.insert(
        "mode".to_owned(),
        Json::Text(pipeline_mode_name(pipeline.mode).to_owned()),
    );
    fields.insert("steps".to_owned(), Json::List(steps));
    Json::Object(fields)
}

const fn pipeline_mode_name(mode: PipelineMode) -> &'static str {
    match mode {
        PipelineMode::PreCall => "pre_call",
        PipelineMode::PostCall => "post_call",
    }
}

const fn pipeline_action_name(action: PipelineAction) -> &'static str {
    match action {
        PipelineAction::Allow => "allow",
        PipelineAction::Block => "block",
        PipelineAction::Next => "next",
        PipelineAction::ModifyResponse => "modify_response",
    }
}
