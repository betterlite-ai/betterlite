#![expect(
    clippy::disallowed_types,
    reason = "a LiteLLM request map carries provider parameters the router does not model"
)]

use std::collections::HashSet;

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod tests;

const SIMPLE_SHUFFLE: &str = "simple-shuffle";
const EXCLUDED_IDS: &str = "_failover_excluded_ids";

// B399 asks for this plan after the order fallbacks and before the generic fallback walk, and B016's selection filters with drop_excluded after order filtering and before the retry skip
#[derive(Debug, Clone, Copy)]
pub struct FailoverInputs<'input> {
    pub strategy: &'input str,
    pub failed_deployment_id: Option<&'input str>,
    pub model_group: &'input str,
    pub deployment_ids: &'input [String],
    pub cooldown_ids: &'input [String],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverEntry {
    pub model: String,
    #[serde(rename = "_excluded_deployment_ids")]
    pub excluded_deployment_ids: Vec<String>,
}

#[must_use]
pub fn plan_weighted_failover(
    inputs: &FailoverInputs<'_>,
    request: &mut Map<String, Value>,
    metadata_key: &str,
) -> Option<FailoverEntry> {
    if inputs.strategy != SIMPLE_SHUFFLE {
        return None;
    }
    let failed_id = inputs
        .failed_deployment_id
        .filter(|deployment_id| !deployment_id.is_empty())?;
    let metadata = request
        .entry(metadata_key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if metadata.is_null() {
        *metadata = Value::Object(Map::new());
    }
    let Value::Object(fields) = metadata else {
        return None;
    };
    let mut excluded = earlier_exclusions(fields);
    excluded.insert(failed_id.to_owned());
    let cooling: HashSet<&str> = inputs.cooldown_ids.iter().map(String::as_str).collect();
    let remaining = inputs.deployment_ids.iter().any(|deployment_id| {
        !cooling.contains(deployment_id.as_str()) && !excluded.contains(deployment_id)
    });
    if !remaining {
        return None;
    }
    let excluded_deployment_ids: Vec<String> = excluded.into_iter().collect();
    fields.insert(
        EXCLUDED_IDS.to_owned(),
        excluded_values(&excluded_deployment_ids),
    );
    Some(FailoverEntry {
        model: inputs.model_group.to_owned(),
        excluded_deployment_ids,
    })
}

// LiteLLM rebuilds this list from a set on every hop, so the first-seen order here is deliberate
fn earlier_exclusions(metadata: &Map<String, Value>) -> IndexSet<String> {
    let Some(Value::Array(entries)) = metadata.get(EXCLUDED_IDS) else {
        return IndexSet::new();
    };
    entries
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn excluded_values(excluded: &[String]) -> Value {
    Value::Array(
        excluded
            .iter()
            .map(|deployment_id| Value::String(deployment_id.clone()))
            .collect(),
    )
}

#[must_use]
pub fn drop_excluded<T, F>(deployments: Vec<T>, excluded: &[String], id_of: F) -> Vec<T>
where
    F: Fn(&T) -> Option<&str>,
{
    if excluded.is_empty() {
        return deployments;
    }
    let dropped: HashSet<&str> = excluded.iter().map(String::as_str).collect();
    deployments
        .into_iter()
        .filter(|deployment| {
            id_of(deployment).is_none_or(|deployment_id| !dropped.contains(deployment_id))
        })
        .collect()
}
