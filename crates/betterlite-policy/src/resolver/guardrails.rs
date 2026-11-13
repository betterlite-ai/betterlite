use std::collections::BTreeSet;

use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::inheritance::resolve_policy_guardrails;
use crate::registry::PolicyRegistry;

use super::matching::matching_policies;

#[must_use]
pub fn resolve_guardrails_for_context(
    context: &PolicyMatchContext,
    registry: &PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<String> {
    let mut guardrails = BTreeSet::new();
    for (policy_name, _) in matching_policies(context, registry, attachments, policy_names) {
        let resolved = resolve_policy_guardrails(policy_name, &registry.policies, Some(context));
        guardrails.extend(resolved.guardrails);
    }
    guardrails.into_iter().collect()
}
