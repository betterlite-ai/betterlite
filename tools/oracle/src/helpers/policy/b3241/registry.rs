use std::collections::BTreeMap;

use betterlite_policy::registry::{
    PolicyRegistry, PolicySource, RawPipeline, RawPolicy, parse_pipeline, parse_policy,
};
use betterlite_policy::values::Policy;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::support;
use crate::helpers::HelperResult;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    LoadPolicies {
        policies: BTreeMap<String, RawPolicy>,
    },
    ParsePolicy {
        policy_name: String,
        policy: RawPolicy,
    },
    ParsePipeline {
        pipeline: RawPipeline,
    },
    Lookup {
        policies: BTreeMap<String, RawPolicy>,
        policy_names: Vec<String>,
    },
    Scenario {
        policies: Option<BTreeMap<String, RawPolicy>>,
        #[serde(default)]
        add: Vec<AddPolicy>,
        #[serde(default)]
        remove: Vec<String>,
    },
}

#[derive(Deserialize)]
pub(crate) struct AddPolicy {
    name: String,
    policy: Policy,
    #[serde(default)]
    source: PolicySource,
}

pub(crate) fn run(action: &Action) -> HelperResult {
    match action {
        Action::LoadPolicies { policies } => {
            let registry = support::loaded(policies)?;
            Ok(snapshot(&registry, &[]))
        }
        Action::ParsePolicy {
            policy_name,
            policy,
        } => parsed_policy(policy_name, policy),
        Action::ParsePipeline { pipeline } => parsed_pipeline(pipeline),
        Action::Lookup {
            policies,
            policy_names,
        } => {
            let registry = support::loaded(policies)?;
            let mut found = Map::new();
            let mut present = Map::new();
            for name in policy_names {
                found.insert(name.clone(), json!(registry.policies.get(name)));
                present.insert(name.clone(), json!(registry.policies.contains_key(name)));
            }
            Ok(json!({ "policies": found, "has": present }))
        }
        Action::Scenario {
            policies,
            add,
            remove,
        } => scenario(policies.as_ref(), add, remove),
    }
}

fn parsed_policy(policy_name: &str, policy: &RawPolicy) -> HelperResult {
    let parsed = parse_policy(policy_name, policy.clone())
        .map_err(|error| support::invalid_input(error.to_string()))?;
    Ok(json!({ "policy": parsed }))
}

fn parsed_pipeline(pipeline: &RawPipeline) -> HelperResult {
    let parsed = parse_pipeline(pipeline.clone())
        .map_err(|error| support::invalid_input(error.to_string()))?;
    Ok(json!({ "pipeline": parsed }))
}

fn scenario(
    policy_config: Option<&BTreeMap<String, RawPolicy>>,
    add: &[AddPolicy],
    remove: &[String],
) -> HelperResult {
    let mut registry = PolicyRegistry::default();
    if let Some(policies) = policy_config {
        registry
            .load_policies(policies.clone())
            .map_err(|error| support::invalid_input(error.to_string()))?;
    }
    for entry in add {
        registry.add_policy(&entry.name, entry.policy.clone(), entry.source);
    }
    let mut removed = Vec::new();
    for name in remove {
        removed.push(registry.remove_policy(name));
    }
    Ok(snapshot(&registry, &removed))
}

fn snapshot(registry: &PolicyRegistry, removed: &[bool]) -> Value {
    let mut names = Vec::new();
    let mut sources = Map::new();
    let mut policies = Map::new();
    for (name, policy) in &registry.policies {
        names.push(name.clone());
        sources.insert(name.clone(), json!(registry.sources.get(name)));
        policies.insert(name.clone(), json!(policy));
    }
    let mut config_policies = Map::new();
    for (name, policy) in &registry.config_policies {
        config_policies.insert(name.clone(), json!(policy));
    }
    json!({
        "initialized": registry.initialized,
        "removed": removed,
        "names": names,
        "sources": sources,
        "policies": policies,
        "config_policies": config_policies,
    })
}
