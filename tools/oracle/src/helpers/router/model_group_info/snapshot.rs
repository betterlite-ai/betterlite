use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};

use super::error::Failure;
use super::input::{Input, Row};

// A Router as the aggregation sees it: the model list under its indices, and the model_group_alias entries.
pub(crate) struct Snapshot {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
}

impl Snapshot {
    // set_model_list: every row as a deployment under the catalog's indices.
    pub(crate) fn build(input: &Input) -> Result<Self, Failure> {
        let mut catalog = DeploymentCatalog::default();
        for row in &input.model_list {
            let spec = spec(row)?;
            catalog.index_patterns(&spec)?;
            catalog.push(spec);
        }
        for index in &input.missing_ids {
            if let Some(row) = catalog.models.get_mut(*index) {
                row.model_info.shift_remove("id");
            }
        }
        Ok(Self {
            catalog,
            aliases: alias_map(&input.model_group_alias),
        })
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
        row.extra.clone(),
        String::new,
    )?)
}

// model_group_alias as get_model_group_info reads it, keeping configuration order.
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
