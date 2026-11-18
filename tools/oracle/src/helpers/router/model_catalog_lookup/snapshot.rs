use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::routing_groups::group::RoutingGroup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use super::error::Failure;
use super::input::{GroupRow, Input, Row};

// The stale entries a stale_index case splices into both indices after the model list is built.
const STALE_ID: &str = "ghost-id";
const STALE_NAME: &str = "ghost";
const STALE_INDEX: usize = 99;

// A Router as model_catalog_lookup sees it: the model list with its indices, the aliases and the routing groups.
pub(crate) struct Snapshot {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
    pub(crate) provider: Box<dyn Fn(&str) -> Option<String>>,
}

impl Snapshot {
    // set_model_list then _init_routing_groups: every row as a deployment under the catalog's indices, then the groups over them.
    pub(crate) fn build(input: &Input) -> Result<Self, Failure> {
        let catalog = catalog_rows(input)?;
        let aliases = alias_map(&input.model_group_alias);
        let groups = routing_group_index(&input.routing_groups, &catalog.model_names, &aliases)?;
        let declared = input.inferred_provider.clone();
        let mut snapshot = Self {
            catalog,
            aliases,
            groups,
            provider: Box::new(move |_model: &str| declared.clone()),
        };
        if input.stale_index {
            snapshot
                .catalog
                .id_index
                .insert(STALE_ID.to_owned(), STALE_INDEX);
            snapshot
                .catalog
                .name_index
                .insert(STALE_NAME.to_owned(), vec![STALE_INDEX]);
        }
        damage_rows(&mut snapshot.catalog, input);
        Ok(snapshot)
    }
}

// set_model_list: every row as a deployment under the catalog's indices, and its name among the known model names.
fn catalog_rows(input: &Input) -> Result<DeploymentCatalog, Failure> {
    let mut catalog = DeploymentCatalog::default();
    for row in &input.model_list {
        let spec = spec(row)?;
        catalog.index_patterns(&spec)?;
        catalog.push(spec);
    }
    for row in &input.model_list {
        catalog.model_names.insert(row.model_name.clone());
    }
    Ok(catalog)
}

// The damaged rows a prebuilt snapshot asked for: an id removed from model_info, and a model_info removed whole.
fn damage_rows(catalog: &mut DeploymentCatalog, input: &Input) {
    for index in &input.missing_ids {
        if let Some(row) = catalog.models.get_mut(*index) {
            row.model_info.shift_remove("id");
        }
    }
    for index in &input.missing_info {
        if let Some(row) = catalog.models.get_mut(*index) {
            row.model_info = JsonObject::new();
        }
    }
}

// set_model_list's per-row identity: the declared id, or the stable one it derives from the name and params.
fn spec(row: &Row) -> Result<DeploymentSpec, Failure> {
    let mut model_info = row.model_info.clone();
    if !model_info.contains_key("id") {
        let generated = generate_model_id(&row.model_name, &row.litellm_params);
        model_info.insert("id".to_owned(), Json::Text(generated));
    }
    Ok(DeploymentSpec::new(
        row.model_name.clone(),
        row.litellm_params.clone(),
        model_info,
        JsonObject::new(),
        String::new,
    )?)
}

// model_group_alias as the lookup reads it, keeping configuration order.
fn alias_map(entries: &JsonObject) -> AliasMap {
    entries
        .iter()
        .map(|(alias, value)| (alias.clone(), alias_target(value)))
        .collect()
}

fn alias_target(value: &Json) -> AliasTarget {
    match value {
        Json::Text(target) => AliasTarget::Name(target.clone()),
        Json::Object(item) => AliasTarget::Item {
            model: item.get("model").cloned(),
            hidden: item.get("hidden").cloned(),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            AliasTarget::Unsupported(value.clone())
        }
    }
}

// _init_routing_groups: the parsed groups indexed against the names and aliases that shadow them.
fn routing_group_index(
    rows: &[GroupRow],
    model_names: &BTreeSet<String>,
    aliases: &AliasMap,
) -> Result<RoutingGroupIndex, Failure> {
    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        groups.push(RoutingGroup::new(
            row.group_name.clone(),
            row.models.clone(),
            &row.routing_strategy,
            None,
            None,
        )?);
    }
    let known: BTreeSet<String> = model_names
        .iter()
        .filter(|name| !name.is_empty())
        .cloned()
        .collect();
    let targets: BTreeMap<String, String> = aliases
        .iter()
        .map(|(alias, target)| {
            (
                alias.clone(),
                target.resolved().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    Ok(RoutingGroupIndex::build(groups, &known, &targets)?.index)
}
