use std::collections::BTreeMap;

use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::matcher::{
    applying_chain_members, policies_matching_conditions, policy_applies,
};
use betterlite_policy::registry::RawPolicy;
use betterlite_policy::scope::{matches_pattern, scope_matches};
use betterlite_policy::values::PolicyScope;
use serde::Deserialize;
use serde_json::{Map, json};

use super::support;
use crate::helpers::HelperResult;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    MatchesPattern {
        value: Option<String>,
        patterns: Vec<String>,
    },
    ScopeMatches {
        scope: PolicyScope,
        context: PolicyMatchContext,
    },
    PolicyApplies {
        policies: BTreeMap<String, RawPolicy>,
        context: PolicyMatchContext,
        policy_names: Vec<String>,
    },
    ApplyingChainMembers {
        policies: BTreeMap<String, RawPolicy>,
        context: PolicyMatchContext,
        policy_name: String,
    },
    MatchingConditions {
        policies: BTreeMap<String, RawPolicy>,
        context: PolicyMatchContext,
        policy_names: Vec<String>,
    },
}

pub(crate) fn run(action: &Action) -> HelperResult {
    match action {
        Action::MatchesPattern { value, patterns } => {
            Ok(json!({ "matches": matches_pattern(value.as_deref(), patterns) }))
        }
        Action::ScopeMatches { scope, context } => {
            Ok(json!({ "matches": scope_matches(scope, context) }))
        }
        Action::PolicyApplies {
            policies,
            context,
            policy_names,
        } => applies(policies, context, policy_names),
        Action::ApplyingChainMembers {
            policies,
            context,
            policy_name,
        } => chain(policies, context, policy_name),
        Action::MatchingConditions {
            policies,
            context,
            policy_names,
        } => conditions(policies, context, policy_names),
    }
}

fn applies(
    policies: &BTreeMap<String, RawPolicy>,
    context: &PolicyMatchContext,
    policy_names: &[String],
) -> HelperResult {
    let registry = support::loaded(policies)?;
    let mut applies = Map::new();
    for name in policy_names {
        applies.insert(
            name.clone(),
            json!(policy_applies(name, context, &registry.policies)),
        );
    }
    Ok(json!({ "applies": applies }))
}

fn chain(
    policies: &BTreeMap<String, RawPolicy>,
    context: &PolicyMatchContext,
    policy_name: &str,
) -> HelperResult {
    let registry = support::loaded(policies)?;
    let members = applying_chain_members(policy_name, context, &registry.policies);
    let applies = policy_applies(policy_name, context, &registry.policies);
    Ok(json!({ "members": members, "applies": applies }))
}

fn conditions(
    policies: &BTreeMap<String, RawPolicy>,
    context: &PolicyMatchContext,
    policy_names: &[String],
) -> HelperResult {
    let registry = support::loaded(policies)?;
    let matching = policies_matching_conditions(policy_names, context, &registry.policies);
    Ok(json!({ "names": matching }))
}
