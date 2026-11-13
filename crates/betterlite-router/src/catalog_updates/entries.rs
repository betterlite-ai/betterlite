use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use crate::deployment_identity::construct::DeploymentInput;
use crate::deployment_identity::stable_id::generate_model_id;

// One model_list entry as the YAML or the database hands it over.
#[derive(Clone, Debug, Deserialize)]
pub struct ModelEntry {
    pub model_name: String,
    pub litellm_params: JsonObject,
    #[serde(default)]
    pub model_info: JsonObject,
    #[serde(flatten)]
    pub extra: JsonObject,
}

fn id_text(model_info: &JsonObject) -> Option<String> {
    match model_info.get("id") {
        Some(Json::Text(id)) => Some(id.clone()),
        Some(Json::Number(id)) => Some(id.to_string()),
        _ => None,
    }
}

// The first half of set_model_list: declared and duplicate ids, generated ids and one input per organization.
#[must_use]
pub fn deployment_inputs(entries: &[ModelEntry]) -> Vec<DeploymentInput> {
    let declared: Vec<String> = entries
        .iter()
        .filter_map(|entry| id_text(&entry.model_info))
        .collect();
    let duplicate_ids: BTreeSet<String> = declared
        .iter()
        .filter(|id| declared.iter().filter(|other| other == id).count() > 1)
        .cloned()
        .collect();
    let mut inputs = Vec::new();
    for entry in entries {
        let mut model_info = entry.model_info.clone();
        let declared_id = id_text(&model_info);
        if !model_info.contains_key("id") {
            let generated = generate_model_id(&entry.model_name, &entry.litellm_params);
            model_info.insert("id".to_owned(), Json::Text(generated));
        }
        let input = DeploymentInput {
            model_name: entry.model_name.clone(),
            litellm_params: entry.litellm_params.clone(),
            model_info,
            extra: entry.extra.clone(),
            declared_id,
            duplicate_ids: duplicate_ids.clone(),
        };
        let Some(Json::List(organizations)) = entry.litellm_params.get("organization") else {
            inputs.push(input);
            continue;
        };
        for organization in organizations {
            let mut per_organization = input.clone();
            per_organization
                .litellm_params
                .insert("organization".to_owned(), organization.clone());
            inputs.push(per_organization);
        }
    }
    inputs
}
