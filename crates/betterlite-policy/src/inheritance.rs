mod tests;

use std::collections::{BTreeMap, BTreeSet};

use crate::condition::evaluate_condition;
use crate::context::{PolicyMatchContext, ResolvedPolicy};
use crate::values::Policy;

#[must_use]
pub fn resolve_inheritance_chain(
    policy_name: &str,
    policies: &BTreeMap<String, Policy>,
) -> Vec<String> {
    let mut visited = BTreeSet::new();
    let mut chain = Vec::new();
    let mut current = policy_name.to_owned();
    loop {
        if visited.contains(&current) {
            tracing::warn!("Circular inheritance detected for policy '{current}'");
            break;
        }
        let Some(policy) = policies.get(&current) else {
            break;
        };
        visited.insert(current.clone());
        chain.push(current.clone());
        let inherit = policy.inherit.as_deref().filter(|name| !name.is_empty());
        let Some(parent_name) = inherit else {
            break;
        };
        parent_name.clone_into(&mut current);
    }
    chain.reverse();
    chain
}

#[must_use]
pub fn resolve_policy_guardrails(
    policy_name: &str,
    policies: &BTreeMap<String, Policy>,
    context: Option<&PolicyMatchContext>,
) -> ResolvedPolicy {
    let inheritance_chain = resolve_inheritance_chain(policy_name, policies);
    let mut guardrails = BTreeSet::new();
    for chain_policy_name in &inheritance_chain {
        let Some(policy) = policies.get(chain_policy_name) else {
            continue;
        };
        let condition_matches = context.is_none_or(|match_context| {
            policy
                .condition
                .as_ref()
                .is_none_or(|condition| evaluate_condition(Some(condition), match_context))
        });
        if !condition_matches {
            tracing::debug!(
                "Policy '{chain_policy_name}' condition did not match, skipping guardrails"
            );
            continue;
        }
        guardrails.extend(policy.guardrails.get_add());
        for guardrail in policy.guardrails.get_remove() {
            guardrails.remove(&guardrail);
        }
    }
    ResolvedPolicy {
        policy_name: policy_name.to_owned(),
        guardrails: guardrails.into_iter().collect(),
        inheritance_chain,
    }
}
