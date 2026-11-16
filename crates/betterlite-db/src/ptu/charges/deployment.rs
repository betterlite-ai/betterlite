use std::borrow::Cow;
use std::collections::BTreeSet;

use betterlite_models::ptu_pricing::{PtuModelInfo, ptu_terms};

use super::decode::{ModelInfo, decode_ptu_model_info, model_info_scalars};
use super::pricing::PtuModel;
use crate::budget_permission_rows::JsonData;

pub struct RouterEntry {
    pub model_name: String,
    pub model_info: JsonData,
}

pub struct StoredModel {
    pub model_id: String,
    pub model_name: String,
    pub model_info: Option<JsonData>,
}

pub struct Deployment {
    pub model_id: String,
    pub model_name: String,
    pub model_info: ModelInfo,
}

pub struct LoadedDeployments {
    pub models: Vec<PtuModel>,
    pub scanned_ids: BTreeSet<String>,
}

pub fn router_deployment(entry: &RouterEntry) -> Option<Deployment> {
    let model_info = decode_ptu_model_info(&entry.model_info)?;
    deployment_of(model_info, &entry.model_name)
}

#[must_use]
pub fn config_deployments(
    entries: &[RouterEntry],
    owned_by_db: &BTreeSet<String>,
) -> Vec<Deployment> {
    entries
        .iter()
        .filter_map(|entry| {
            let model_info = decode_ptu_model_info(&entry.model_info)?;
            if is_credential_copy(&model_info) {
                return None;
            }
            deployment_of(model_info, &entry.model_name)
                .filter(|deployment| !owned_by_db.contains(&deployment.model_id))
        })
        .collect()
}

pub fn parse_ptu_model(
    model_id: &str,
    model_name: &str,
    model_info: &ModelInfo,
) -> Option<PtuModel> {
    let scalars = model_info_scalars(model_info);
    let terms = ptu_terms(&PtuModelInfo::from(&scalars))?;
    let declared_name = model_info.get("team_public_model_name");
    let public_name = match declared_name {
        Some(JsonData::String(name)) if !name.is_empty() => name.as_str(),
        _ => model_name,
    };
    Some(PtuModel {
        model_id: model_id.to_owned(),
        model_name: public_name.to_owned(),
        team_id: terms.team_id,
        ptu_count: terms.ptu_count,
        cost_per_ptu_per_hour: terms.cost_per_ptu_per_hour,
        effective_from: terms.effective_from,
        effective_to: terms.effective_to,
    })
}

#[must_use]
pub fn scan_deployments(rows: &[StoredModel], entries: &[RouterEntry]) -> LoadedDeployments {
    let owned_by_db: BTreeSet<String> = rows
        .iter()
        .map(|row| row.model_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    let cloned = config_deployments(entries, &owned_by_db);
    let mut models: Vec<PtuModel> = rows
        .iter()
        .filter_map(|row| {
            let raw = row.model_info.as_ref()?;
            let model_info = decode_ptu_model_info(raw)?;
            parse_ptu_model(&row.model_id, &row.model_name, &model_info)
        })
        .collect();
    models.extend(cloned.iter().filter_map(|deployment| {
        parse_ptu_model(
            &deployment.model_id,
            &deployment.model_name,
            &deployment.model_info,
        )
    }));
    let mut scanned_ids = owned_by_db;
    scanned_ids.extend(cloned.iter().map(|deployment| deployment.model_id.clone()));
    scanned_ids.extend(models.iter().map(|model| model.model_id.clone()));
    LoadedDeployments {
        models,
        scanned_ids,
    }
}

fn deployment_of(model_info: Cow<'_, ModelInfo>, model_name: &str) -> Option<Deployment> {
    let JsonData::String(model_id) = model_info.get("id")? else {
        return None;
    };
    if model_id.is_empty() {
        return None;
    }
    Some(Deployment {
        model_id: model_id.clone(),
        model_name: model_name.to_owned(),
        model_info: model_info.into_owned(),
    })
}

fn is_credential_copy(model_info: &ModelInfo) -> bool {
    if model_info.get("db_model") == Some(&JsonData::Bool(true)) {
        return true;
    }
    model_info
        .get("original_model_id")
        .is_some_and(|id| !matches!(id, JsonData::Null))
}
