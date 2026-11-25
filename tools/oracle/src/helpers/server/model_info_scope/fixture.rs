use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_server::model_info_scope::compose::Deployment;
use betterlite_server::model_info_scope::teams::TeamModelLists;
use betterlite_server::model_visibility::testing::{Row as SnapshotRow, Snapshot};
use serde_json::{Value, json};

use super::Failure;
use super::records::{Input, RawRow};

pub(super) fn fixture(input: &Input) -> Result<Snapshot, Failure> {
    let infos: Vec<Json> = input
        .model_list
        .iter()
        .map(|row| Json::Object(row.model_info.clone()))
        .collect();
    let rows: Vec<SnapshotRow<'_>> = input
        .model_list
        .iter()
        .zip(&infos)
        .map(|(row, model_info)| SnapshotRow {
            model_name: &row.model_name,
            litellm_params: &row.litellm_params,
            model_info,
        })
        .collect();
    let aliases: JsonObject = input
        .model_group_alias
        .iter()
        .map(|(alias, target)| (alias.clone(), Json::Text(target.clone())))
        .collect();
    Ok(Snapshot::build(&rows, &aliases)?)
}

pub(super) fn specs(rows: &[RawRow]) -> Vec<Deployment> {
    rows.iter()
        .map(|row| Deployment {
            model_name: row.model_name.clone(),
            litellm_params: row.litellm_params.clone(),
            model_info: row.model_info.clone(),
            extra: JsonObject::new(),
        })
        .collect()
}

pub(super) fn spec_ids(models: &[Deployment]) -> Value {
    let ids: Vec<Value> = models
        .iter()
        .map(|model| {
            model
                .id()
                .map_or(Value::Null, |id| Value::String(id.to_owned()))
        })
        .collect();
    json!({"ids": ids})
}

pub(super) fn team_map(found: &TeamModelLists) -> Value {
    let mut sorted: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (model_id, team_ids) in found {
        let mut names: Vec<&str> = team_ids.iter().map(String::as_str).collect();
        names.sort_unstable();
        sorted.insert(model_id.as_str(), names);
    }
    json!({"team_models": sorted})
}
