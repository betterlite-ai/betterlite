mod efforts;
mod groups;

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_router::router_predicates::JsonMap;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub groups: Option<Vec<JsonMap>>,
    pub deployments: Vec<DeploymentInput>,
    pub aliases: BTreeMap<String, String>,
    pub names: Vec<String>,
    pub cost_map: JsonMap,
    pub efforts: Vec<EffortInput>,
    pub intersections: Vec<IntersectionInput>,
    pub rounds: Vec<RoundInput>,
    pub lookups: Vec<ModelLookup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentInput {
    pub model_name: String,
    pub identifier: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffortInput {
    pub model_info: JsonMap,
    pub resolved_model_info: Option<JsonMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntersectionInput {
    pub current: Option<Vec<String>>,
    pub resolved: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundInput {
    pub requested: String,
    pub declared: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLookup {
    pub model: String,
    pub custom_llm_provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub groups: Vec<GroupRow>,
    pub error: Option<String>,
    pub names: Vec<NameFacts>,
    pub efforts: Vec<Option<Vec<String>>>,
    pub intersections: Vec<Option<Vec<String>>>,
    pub rounds: Vec<String>,
    pub declared: Vec<Option<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRow {
    pub group_name: String,
    pub models: Vec<String>,
    pub routing_strategy: String,
    pub routing_strategy_args: Option<JsonMap>,
    pub model_priorities: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameFacts {
    pub name: String,
    pub is_callable: bool,
    pub priority: bool,
    pub alternatives: bool,
    pub ids: Option<Vec<String>>,
}

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let specs = groups::deployment_specs(&input.deployments)?;
    let cost_map = ModelCostMap::parse(&serde_json::to_vec(&input.cost_map).map_err(invalid)?)
        .map_err(invalid)?;
    let mut output = efforts::compute(input, &cost_map)?;
    match groups::group_index(input) {
        Ok((index, rows)) => {
            output.groups = rows;
            output.names = input
                .names
                .iter()
                .map(|name| groups::name_facts(&index, &specs, name))
                .collect();
        }
        Err(error) => output.error = Some(error.to_string()),
    }
    Ok(ProbeOutput::RoutingGroups(output))
}
