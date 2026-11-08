use std::collections::BTreeSet;

use crate::attachment::AttachmentRegistry;
use crate::condition::evaluate_condition;
use crate::context::PolicyMatchContext;
use crate::pipeline::GuardrailPipeline;
use crate::registry::PolicyRegistry;

use super::matching::matching_policies;

#[must_use]
pub fn resolve_pipelines_for_context(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<(String, GuardrailPipeline)> {
    let mut pipelines = Vec::new();
    for (policy_name, policy) in matching_policies(context, registry, attachments, policy_names) {
        if !evaluate_condition(policy.condition.as_ref(), context) {
            tracing::debug!("Policy '{policy_name}' condition did not match, skipping pipeline");
            continue;
        }
        if let Some(pipeline) = &policy.pipeline {
            tracing::debug!(
                "Policy '{policy_name}' has a pipeline with {} steps",
                pipeline.steps.len()
            );
            pipelines.push((policy_name.clone(), pipeline.clone()));
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
