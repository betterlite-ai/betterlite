mod bucket;
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::matcher::{policies_matching_conditions, policy_applies};
use crate::pipeline::json_value::JsonValue;
use crate::pipeline::{GuardrailPipeline, PipelineMode};
use crate::registry::PolicyRegistry;
use crate::resolver::pipelines::resolve_pipelines_for_context;

pub type PolicyPipelines = Vec<(String, GuardrailPipeline)>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetadataBucket {
    #[serde(
        default,
        rename = "_guardrail_pipelines",
        skip_serializing_if = "Option::is_none"
    )]
    pub guardrail_pipelines: Option<PolicyPipelines>,
    #[serde(
        default,
        rename = "_pipeline_managed_guardrails",
        skip_serializing_if = "Option::is_none"
    )]
    pub pipeline_managed_guardrails: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_policies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_guardrails: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_sources: Option<BTreeMap<String, String>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, JsonValue>,
}

// LiteLLM picks the bucket by whether the key is there, and a null bucket is replaced by one
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RetrievalData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub metadata: Option<Option<MetadataBucket>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub litellm_metadata: Option<Option<MetadataBucket>>,
    #[serde(flatten)]
    pub passthrough: BTreeMap<String, JsonValue>,
}

impl RetrievalData {
    fn bucket_mut(&mut self) -> &mut MetadataBucket {
        let slot = if self.litellm_metadata.is_some() {
            &mut self.litellm_metadata
        } else {
            &mut self.metadata
        };
        slot.get_or_insert_with(|| Some(MetadataBucket::default()))
            .get_or_insert_with(MetadataBucket::default)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UngovernedReason {
    NoRouter,
    ResponseIdNamesNoDeployment,
    DeploymentNoLongerInRouter,
}

impl UngovernedReason {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoRouter => "no router",
            Self::ResponseIdNamesNoDeployment => "response id names no deployment",
            Self::DeploymentNoLongerInRouter => "deployment no longer in the router",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UngovernedRetrieval {
    pub reason: UngovernedReason,
}

// betterlite-policy may not depend on the router crate, so the caller reads the router through this port
pub trait PolicyRouterView: Send + Sync {
    fn deployment_model_name(&self, model_id: &str) -> Option<String>;

    fn model_group_aliases(&self) -> Vec<(String, String)>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RetrievalAuth<'a> {
    pub team_alias: Option<&'a str>,
    pub key_alias: Option<&'a str>,
}

pub struct RetrievalRequest<'a> {
    pub auth: RetrievalAuth<'a>,
    pub tags: Vec<String>,
}

pub struct RetrievalDependencies<'a> {
    pub registry: &'a PolicyRegistry,
    pub attachments: &'a AttachmentRegistry,
    pub router: Option<&'a dyn PolicyRouterView>,
    pub model_id_from_response_id: &'a dyn Fn(Option<&str>) -> Option<String>,
}

#[derive(Debug, Default)]
pub struct PostCallPipelines {
    pub pipelines: PolicyPipelines,
    pub sources: BTreeMap<String, String>,
}

#[must_use]
pub fn model_group_for_response_id(
    response_id: Option<&str>,
    router: Option<&dyn PolicyRouterView>,
    model_id_from_response_id: &dyn Fn(Option<&str>) -> Option<String>,
) -> Result<String, UngovernedRetrieval> {
    let Some(router) = router else {
        return Err(UngovernedRetrieval {
            reason: UngovernedReason::NoRouter,
        });
    };
    let Some(model_id) = model_id_from_response_id(response_id) else {
        return Err(UngovernedRetrieval {
            reason: UngovernedReason::ResponseIdNamesNoDeployment,
        });
    };
    let Some(model_name) = router.deployment_model_name(&model_id) else {
        return Err(UngovernedRetrieval {
            reason: UngovernedReason::DeploymentNoLongerInRouter,
        });
    };
    if let Some(hidden_by) = submit_model_hidden_by(&model_name, &router.model_group_aliases()) {
        tracing::warn!(
            "Policy engine: background response {} re-matches policies on retrieval as model group {} ({}), so a policy attached to the model name it was submitted as does not run on it",
            response_id.unwrap_or("None"),
            model_name,
            hidden_by
        );
    }
    Ok(model_name)
}

#[must_use]
pub fn submit_model_hidden_by(model_group: &str, aliases: &[(String, String)]) -> Option<String> {
    if model_group.contains('*') {
        return Some("a wildcard deployment".to_owned());
    }
    let hidden_by: Vec<&str> = aliases
        .iter()
        .filter(|(_, target)| target.as_str() == model_group)
        .map(|(alias, _)| alias.as_str())
        .collect();
    if hidden_by.is_empty() {
        return None;
    }
    Some(format!(
        "the target of model_group_alias {}",
        hidden_by.join(", ")
    ))
}

#[must_use]
pub fn retrieval_context(
    auth: &RetrievalAuth<'_>,
    model_group: &str,
    tags: Vec<String>,
) -> PolicyMatchContext {
    PolicyMatchContext {
        team_alias: auth.team_alias.map(str::to_owned),
        key_alias: auth.key_alias.map(str::to_owned),
        model: Some(model_group.to_owned()),
        tags: (!tags.is_empty()).then_some(tags),
    }
}

#[must_use]
pub fn post_call_pipelines(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
) -> PostCallPipelines {
    let applies = |policy_name: &str| policy_applies(policy_name, context, &registry.policies);
    let matches = attachments.attached_policy_matches(context, Some(&applies));
    if matches.is_empty() {
        return PostCallPipelines::default();
    }
    let policy_names: Vec<String> = matches
        .iter()
        .map(|found| found.policy_name.clone())
        .collect();
    let applied = policies_matching_conditions(&policy_names, context, &registry.policies);
    let pipelines = resolve_pipelines_for_context(context, registry, attachments, Some(&applied))
        .into_iter()
        .filter(|(_, pipeline)| pipeline.mode == PipelineMode::PostCall)
        .collect();
    let sources = matches
        .into_iter()
        .map(|found| (found.policy_name, found.matched_via))
        .collect();
    PostCallPipelines { pipelines, sources }
}

pub fn attach_retrieval_pipelines(
    data: &mut RetrievalData,
    request: &RetrievalRequest<'_>,
    deps: &RetrievalDependencies<'_>,
) {
    if !deps.registry.initialized {
        return;
    }
    let Some(model_group) = governed_model_group(data, deps) else {
        return;
    };
    let context = retrieval_context(&request.auth, &model_group, request.tags.clone());
    let found = post_call_pipelines(&context, deps.registry, deps.attachments);
    let Some(added) = bucket::attach_new_pipelines(data, &found.pipelines, &found.sources) else {
        return;
    };
    log_attached(data, &model_group, &added);
}

fn governed_model_group(data: &RetrievalData, deps: &RetrievalDependencies<'_>) -> Option<String> {
    match model_group_for_response_id(
        data.response_id.as_deref(),
        deps.router,
        deps.model_id_from_response_id,
    ) {
        Ok(model_group) => Some(model_group),
        Err(ungoverned) => {
            tracing::warn!(
                "Policy engine: background response {} is retrieved without its post_call policy pipelines ({})",
                data.response_id.as_deref().unwrap_or("None"),
                ungoverned.reason.message()
            );
            None
        }
    }
}

fn log_attached(data: &RetrievalData, model_group: &str, added: &[(String, GuardrailPipeline)]) {
    let names = added
        .iter()
        .map(|(policy_name, _)| policy_name.as_str())
        .collect::<Vec<&str>>()
        .join(", ");
    tracing::debug!(
        "Policy engine: attached post_call pipelines to the retrieval of background response {} (model group {}): {}",
        data.response_id.as_deref().unwrap_or("None"),
        model_group,
        names
    );
}
