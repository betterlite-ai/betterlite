use std::collections::BTreeSet;
use std::mem::take;

use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_identity::stable_id::generate_model_id;
use crate::model_catalog_lookup::cache::{CatalogCaches, Invalidation};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::fields::text;

// Router._build_model_name_index: the name and team indices again from the rows, leaving the id map to its own builder.
pub fn rebuild_names(catalog: &mut DeploymentCatalog) {
    catalog.name_index.clear();
    catalog.team_index.clear();
    let rows = take(&mut catalog.models);
    for (index, row) in rows.iter().enumerate() {
        if !row.model_name.is_empty() {
            catalog
                .name_index
                .entry(row.model_name.clone())
                .or_default()
                .push(index);
        }
        index_team(catalog, row, index);
    }
    catalog.models = rows;
}

// Router._update_team_model_index: a team's public name indexes the row even when the row has no model name.
fn index_team(catalog: &mut DeploymentCatalog, row: &DeploymentSpec, index: usize) {
    let team = text(&row.model_info, "team_id").filter(|value| !value.is_empty());
    let public_name =
        text(&row.model_info, "team_public_model_name").filter(|value| !value.is_empty());
    if let (Some(team), Some(public_name)) = (team, public_name) {
        catalog
            .team_index
            .entry((team.to_owned(), public_name.to_owned()))
            .or_default()
            .push(index);
    }
}

// Router._build_model_id_to_deployment_index_map: the rows, the id map and both caches again from the given list.
pub fn rebuild_ids(
    catalog: &mut DeploymentCatalog,
    rows: Vec<DeploymentSpec>,
    caches: &CatalogCaches<'_>,
) -> Result<(), LookupError> {
    catalog.models.clear();
    caches.invalidate(Invalidation::ModelMetadata);
    caches.invalidate(Invalidation::AccessGroups);
    for (index, mut row) in rows.into_iter().enumerate() {
        match row.model_info.get("id") {
            None | Some(Json::Null) => {
                let id = generate_model_id(&row.model_name, &row.litellm_params);
                row.model_info.insert("id".to_owned(), Json::Text(id));
            }
            Some(Json::Text(_)) => {}
            Some(_) => return Err(LookupError::InvalidId { index }),
        }
        catalog.push(row);
    }
    Ok(())
}

// Router._deployment_ids: every present id as Python text, collected into a set.
#[must_use]
pub fn deployment_ids(deployments: &[DeploymentSpec]) -> BTreeSet<String> {
    deployments
        .iter()
        .filter_map(|deployment| deployment.model_info.get("id"))
        .filter_map(id_text)
        .collect()
}

// Python's str() of the scalar an id can hold; a null id never reaches this and a list or object is no id at all.
fn id_text(id: &Json) -> Option<String> {
    match id {
        Json::Text(value) => Some(value.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}
