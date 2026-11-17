use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::routing_groups::error::RoutingGroupError;
use betterlite_router::routing_groups::group::parse_routing_groups;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::strategy_dispatch::groups::GroupStrategy;

use crate::router_probes::error::OperationError;
use crate::sdk_http::messages_stream_logging::outcome::Flag;

use super::{DeploymentInput, GroupRow, Input, NameFacts, invalid};

pub(super) fn deployment_specs(
    entries: &[DeploymentInput],
) -> Result<Vec<DeploymentSpec>, OperationError> {
    entries
        .iter()
        .map(|entry| {
            let mut params = JsonMap::new();
            params.insert("model".to_owned(), Json::Text(entry.model.clone()));
            let mut info = JsonMap::new();
            info.insert("id".to_owned(), Json::Text(entry.identifier.clone()));
            DeploymentSpec::new(
                entry.model_name.clone(),
                params,
                info,
                JsonMap::new(),
                String::new,
            )
            .map_err(invalid)
        })
        .collect()
}

pub(super) fn group_index(
    input: &Input,
) -> Result<(RoutingGroupIndex, Vec<GroupRow>), RoutingGroupError> {
    let known: BTreeSet<String> = input
        .deployments
        .iter()
        .map(|deployment| deployment.model_name.clone())
        .collect();
    let parsed = parse_routing_groups(input.groups.as_deref(), &known)?;
    let rows: Vec<GroupRow> = parsed
        .groups
        .iter()
        .map(|group| GroupRow {
            group_name: group.group_name.clone(),
            models: group.models.clone(),
            routing_strategy: match group.routing_strategy {
                GroupStrategy::Priority => "priority",
                GroupStrategy::Routing(routing) => routing.wire_name(),
            }
            .to_owned(),
            routing_strategy_args: group.routing_strategy_args.clone(),
            model_priorities: group.model_priorities.clone(),
        })
        .collect();
    let built = RoutingGroupIndex::build(parsed.groups, &known, &input.aliases)?;
    Ok((built.index, rows))
}

fn deploys(specs: &[DeploymentSpec], member: &str) -> Vec<DeploymentSpec> {
    specs
        .iter()
        .filter(|spec| spec.model_name == member)
        .cloned()
        .collect()
}

pub(super) fn name_facts(
    index: &RoutingGroupIndex,
    specs: &[DeploymentSpec],
    name: &str,
) -> NameFacts {
    let callable = index.callable_group(name).is_some();
    let ids: Option<Vec<String>> = if callable {
        let rows = index.rows(Some(name), &|member| deploys(specs, member));
        let found: BTreeSet<String> = rows
            .iter()
            .filter_map(|row| row.id().map(str::to_owned))
            .collect();
        Some(found.into_iter().collect())
    } else {
        None
    };
    NameFacts {
        name: name.to_owned(),
        is_callable: Flag(callable),
        priority: Flag(index.is_priority_group(name)),
        alternatives: Flag(index.has_alternatives(name, &|member| deploys(specs, member).len())),
        ids,
    }
}
