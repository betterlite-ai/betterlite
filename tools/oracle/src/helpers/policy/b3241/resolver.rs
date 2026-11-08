use std::collections::BTreeMap;

use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::registry::{PolicyRegistry, RawPolicy};
use betterlite_policy::resolver::guardrails::resolve_guardrails_for_context;
use betterlite_policy::resolver::pipelines::{
    pipeline_managed_guardrails, resolve_pipelines_for_context,
};
use betterlite_policy::values::PolicyAttachment;
use serde::Deserialize;
use serde_json::json;

use super::support;
use crate::helpers::{HelperError, HelperResult};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    ResolveGuardrails {
        policies: BTreeMap<String, RawPolicy>,
        #[serde(default)]
        attachments: Vec<PolicyAttachment>,
        context: PolicyMatchContext,
        policy_names: Option<Vec<String>>,
    },
    ResolvePipelines {
        policies: BTreeMap<String, RawPolicy>,
        #[serde(default)]
        attachments: Vec<PolicyAttachment>,
        context: PolicyMatchContext,
        policy_names: Option<Vec<String>>,
    },
    ManagedGuardrails {
        pipelines: Vec<(String, GuardrailPipeline)>,
    },
}

pub(crate) fn run(action: &Action) -> HelperResult {
    match action {
        Action::ResolveGuardrails {
            policies,
            attachments,
            context,
            policy_names,
        } => {
            let (registry, attachment_registry) = registries(policies, attachments)?;
            let resolved = resolve_guardrails_for_context(
                context,
                &registry,
                &attachment_registry,
                policy_names.as_deref(),
            );
            Ok(json!({ "guardrails": resolved }))
        }
        Action::ResolvePipelines {
            policies,
            attachments,
            context,
            policy_names,
        } => {
            let (registry, attachment_registry) = registries(policies, attachments)?;
            let resolved = resolve_pipelines_for_context(
                context,
                &registry,
                &attachment_registry,
                policy_names.as_deref(),
            );
            Ok(json!({ "pipelines": resolved }))
        }
        Action::ManagedGuardrails { pipelines } => Ok(json!({
            "guardrails": pipeline_managed_guardrails(pipelines),
        })),
    }
}

fn registries(
    policies: &BTreeMap<String, RawPolicy>,
    attachments: &[PolicyAttachment],
) -> Result<(PolicyRegistry, AttachmentRegistry), Box<HelperError>> {
    let mut policy_registry = PolicyRegistry::default();
    policy_registry
        .load_policies(policies.clone())
        .map_err(|error| support::invalid_input(error.to_string()))?;
    let mut attachment_registry = AttachmentRegistry::default();
    attachment_registry.load_attachments(attachments.to_vec());
    Ok((policy_registry, attachment_registry))
}
