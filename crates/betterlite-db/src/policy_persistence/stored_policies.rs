use std::collections::BTreeMap;

use betterlite_policy::records::db::PolicyDBResponse;
use betterlite_policy::registry::{
    RawGuardrailLists, RawGuardrails, RawPipeline, RawPolicy, parse_policy,
};
use betterlite_policy::row_adapter::PolicyTableSource;
use betterlite_policy::values::{ModelPatterns, Policy, PolicyCondition};
use serde::Deserialize;

use super::PolicyError;
use super::error::Operation;

// LiteLLM reads only the model key of a stored condition
#[derive(Deserialize)]
struct StoredCondition {
    #[serde(default)]
    model: Option<ModelPatterns>,
}

#[derive(Deserialize)]
struct StoredPolicy {
    inherit: Option<String>,
    description: Option<String>,
    guardrails_add: Vec<String>,
    guardrails_remove: Vec<String>,
    condition: Option<StoredCondition>,
    pipeline: Option<RawPipeline>,
}

pub(super) fn parse_stored_policy<Client>(
    response: &PolicyDBResponse,
    operation: Operation,
) -> Result<Policy, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let chain = || vec![operation];
    let stored = serde_json::to_string(response)
        .and_then(|text| serde_json::from_str::<StoredPolicy>(&text))
        .map_err(|source| PolicyError::<Client>::Json(chain(), source).logged())?;
    let raw = RawPolicy {
        inherit: stored.inherit,
        description: stored.description,
        guardrails: Some(RawGuardrails::Lists(RawGuardrailLists {
            add: Some(stored.guardrails_add),
            remove: Some(stored.guardrails_remove),
        })),
        condition: stored.condition.map(|condition| PolicyCondition {
            model: condition.model,
        }),
        pipeline: stored.pipeline,
    };
    parse_policy(&response.policy_name, raw)
        .map_err(|source| PolicyError::<Client>::Policy(chain(), source).logged())
}

pub(super) fn policy_map<Client>(
    responses: &[PolicyDBResponse],
    operation: Operation,
) -> Result<BTreeMap<String, Policy>, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let mut policies = BTreeMap::new();
    for response in responses {
        let policy = parse_stored_policy::<Client>(response, operation)?;
        policies.insert(response.policy_name.clone(), policy);
    }
    Ok(policies)
}
