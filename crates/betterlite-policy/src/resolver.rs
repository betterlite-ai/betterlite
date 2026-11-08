mod tests;

use std::collections::BTreeSet;

use crate::attachment::AttachmentRegistry;
use crate::condition::evaluate_condition;
use crate::context::PolicyMatchContext;
use crate::inheritance::resolve_policy_guardrails;
use crate::matcher::get_matching_policies;
use crate::pipeline::GuardrailPipeline;
use crate::registry::PolicyRegistry;

fn matching_policy_names(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<String> {
    policy_names.map_or_else(
        || get_matching_policies(context, attachments, registry),
        <[String]>::to_vec,
    )
}

#[must_use]
pub fn resolve_guardrails_for_context(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<String> {
    if !registry.initialized {
        return Vec::new();
    }
    let matching = matching_policy_names(context, registry, attachments, policy_names);
    if matching.is_empty() {
        tracing::debug!(
            team_alias = ?context.team_alias,
            key_alias = ?context.key_alias,
            model = ?context.model,
            "no policies match the context"
        );
        return Vec::new();
    }
    let mut guardrails = BTreeSet::new();
    for policy_name in &matching {
        let resolved = resolve_policy_guardrails(policy_name, &registry.policies, Some(context));
        guardrails.extend(resolved.guardrails);
    }
    guardrails.into_iter().collect()
}

#[must_use]
pub fn resolve_pipelines_for_context(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<(String, GuardrailPipeline)> {
    if !registry.initialized {
        return Vec::new();
    }
    let matching = matching_policy_names(context, registry, attachments, policy_names);
    let mut pipelines = Vec::new();
    for policy_name in matching {
        let Some(policy) = registry.policies.get(&policy_name) else {
            continue;
        };
        if !evaluate_condition(policy.condition.as_ref(), context) {
            tracing::debug!("Policy '{policy_name}' condition did not match, skipping pipeline");
            continue;
        }
        if let Some(pipeline) = &policy.pipeline {
            tracing::debug!(
                "Policy '{policy_name}' has a pipeline with {} steps",
                pipeline.steps.len()
            );
            pipelines.push((policy_name, pipeline.clone()));
        }
    }
    pipelines
}

#[must_use]
pub fn pipeline_managed_guardrails(pipelines: &[(String, GuardrailPipeline)]) -> BTreeSet<String> {
    let mut managed = BTreeSet::new();
    for (_, pipeline) in pipelines {
        for step in &pipeline.steps {
            managed.insert(step.guardrail.clone());
        }
    }
    managed
}
