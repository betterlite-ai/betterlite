use std::collections::BTreeSet;

use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::AccessGroups;

use super::error::ScopeError;

pub fn text_models(values: &[JsonData], field: &'static str) -> Result<Vec<String>, ScopeError> {
    let mut names = Vec::with_capacity(values.len());
    for value in values {
        let JsonData::String(name) = value else {
            return Err(ScopeError::InvalidGrant { field });
        };
        names.push(name.clone());
    }
    Ok(names)
}

#[must_use]
pub fn non_team_model_ids(lookup: &CatalogLookup<'_>) -> Vec<String> {
    lookup
        .get_model_ids(None, true)
        .into_iter()
        .filter_map(|id| match id {
            Json::Text(text) => Some(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

fn granted_ids(name: &str, lookup: &CatalogLookup<'_>) -> Result<Vec<String>, ScopeError> {
    Ok(lookup
        .get_model_list(Some(name), None)?
        .iter()
        .filter_map(|deployment| deployment.id().map(str::to_owned))
        .collect())
}

pub fn grant_deployment_ids(
    grant: &[String],
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<Vec<String>, ScopeError> {
    if grant.is_empty() || grant.iter().any(|model| model == ALL_PROXY_MODELS) {
        return Ok(non_team_model_ids(lookup));
    }
    let groups = access_groups.get_model_access_groups(lookup, None, None, None)?;
    let mut ids = Vec::new();
    for model in grant {
        ids.extend(granted_ids(model, lookup)?);
        for member in groups.get(model).map_or(&[][..], Vec::as_slice) {
            ids.extend(granted_ids(member, lookup)?);
        }
    }
    Ok(ids)
}

pub fn direct_access_models(
    user_models: &[String],
    key_models: &[String],
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<Vec<String>, ScopeError> {
    let user_ids = grant_deployment_ids(user_models, lookup, access_groups)?;
    if key_models.is_empty() || key_models.iter().any(|model| model == ALL_PROXY_MODELS) {
        return Ok(user_ids);
    }
    let key_ids: BTreeSet<String> = grant_deployment_ids(key_models, lookup, access_groups)?
        .into_iter()
        .collect();
    Ok(user_ids
        .into_iter()
        .filter(|model_id| key_ids.contains(model_id))
        .collect())
}
