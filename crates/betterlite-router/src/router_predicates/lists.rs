use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use super::JsonMap;
use super::source_text::source_text;

#[must_use]
pub fn deployment_ids(deployments: &[JsonMap]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for deployment in deployments {
        if let Some(id) = deployment_id(deployment) {
            ids.insert(id);
        }
    }
    ids
}

#[must_use]
pub fn config_deployments(model_list: &[Json]) -> Vec<&JsonMap> {
    let mut deployments = Vec::new();
    for row in model_list {
        if let Some(deployment) = config_row(row) {
            deployments.push(deployment);
        }
    }
    deployments
}

fn deployment_id(deployment: &JsonMap) -> Option<String> {
    let Some(Json::Object(model_info)) = deployment.get("model_info") else {
        return None;
    };
    match model_info.get("id") {
        Some(Json::Null) | None => None,
        Some(id) => Some(source_text(id)),
    }
}

fn config_row(row: &Json) -> Option<&JsonMap> {
    let Json::Object(fields) = row else {
        return None;
    };
    match fields.get("model_info") {
        Some(Json::Object(info)) if matches!(info.get("db_model"), Some(Json::Bool(true))) => None,
        _ => Some(fields),
    }
}
