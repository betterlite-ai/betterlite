mod tests;

use std::collections::BTreeMap;

use crate::attachment::AttachmentRegistry;
use crate::condition::evaluate_condition;
use crate::context::PolicyMatchContext;
use crate::inheritance::resolve_inheritance_chain;
use crate::registry::PolicyRegistry;
use crate::values::Policy;

#[must_use]
pub fn get_matching_policies(
    context: &PolicyMatchContext,
    attachments: &AttachmentRegistry,
    registry: &PolicyRegistry,
) -> Vec<String> {
    if !attachments.initialized {
        tracing::debug!("AttachmentRegistry not initialized, returning empty list");
        return Vec::new();
    }
    let uninitialized = BTreeMap::new();
    let policies = if registry.initialized {
        &registry.policies
    } else {
        &uninitialized
    };
    let applies = |policy_name: &str| policy_applies(policy_name, context, policies);
    attachments.get_attached_policies(context, Some(&applies))
}

#[must_use]
pub fn applying_chain_members(
    policy_name: &str,
    context: &PolicyMatchContext,
    policies: &BTreeMap<String, Policy>,
) -> Vec<String> {
    let chain = resolve_inheritance_chain(policy_name, policies);
    let mut applying = Vec::new();
    for name in chain {
        if let Some(policy) = policies.get(&name)
            && evaluate_condition(policy.condition.as_ref(), context)
        {
            applying.push(name);
        }
    }
    applying
}

#[must_use]
pub fn policy_applies(
    policy_name: &str,
    context: &PolicyMatchContext,
    policies: &BTreeMap<String, Policy>,
) -> bool {
    let applying = applying_chain_members(policy_name, context, policies);
    if let Some(ancestor) = applying.first()
        && !applying.iter().any(|name| name == policy_name)
    {
        tracing::info!(
            policy = policy_name,
            ancestor = ancestor.as_str(),
            team_alias = ?context.team_alias,
            key_alias = ?context.key_alias,
            model = ?context.model,
            "policy applied through an ancestor although its own condition did not match"
        );
    }
    !applying.is_empty()
}

#[must_use]
pub fn policies_matching_conditions(
    policy_names: &[String],
    context: &PolicyMatchContext,
    policies: &BTreeMap<String, Policy>,
) -> Vec<String> {
    let mut matching = Vec::new();
    for policy_name in policy_names {
        if !applying_chain_members(policy_name, context, policies).is_empty() {
            matching.push(policy_name.clone());
        }
    }
    matching
}
