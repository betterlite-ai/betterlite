use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_model_patterns::ALL_TEAM_MODELS;
use crate::auth_model_patterns::view::ModelAccessView;
use crate::auth_request_models::RequestedModels;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelGrantPattern {
    pub models: Vec<String>,
    pub team_model_aliases: BTreeMap<String, String>,
    pub key_model_aliases: BTreeMap<String, String>,
    pub team_id: Option<String>,
}

#[must_use]
pub fn resolve_key_models(
    models: &[String],
    team_models: &[String],
    team_id: Option<&str>,
) -> Vec<String> {
    if !models.iter().any(|model| model.as_str() == ALL_TEAM_MODELS) {
        return models.to_vec();
    }
    if team_id.is_none() {
        return Vec::new();
    }
    team_models.to_vec()
}

// alias_map keeps every string target and drops the rest, whatever the map holds
#[must_use]
pub fn key_model_aliases(aliases: &BTreeMap<String, JsonData>) -> BTreeMap<String, String> {
    let mut decoded = BTreeMap::new();
    for (alias, target) in aliases {
        if let JsonData::String(value) = target {
            decoded.insert(alias.clone(), value.clone());
        }
    }
    decoded
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn model_in_team_aliases(model: &str, team_model_aliases: &BTreeMap<String, String>) -> bool {
    team_model_aliases.contains_key(model)
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn team_alias_target<'aliases>(
    model: &'aliases str,
    team_model_aliases: &'aliases BTreeMap<String, String>,
) -> &'aliases str {
    team_model_aliases.get(model).map_or(model, String::as_str)
}

#[must_use]
pub fn live_team_alias_target(
    model: &str,
    team_model_aliases: &BTreeMap<String, String>,
    team_id: Option<&str>,
    view: Option<&dyn ModelAccessView>,
) -> String {
    let Some(target) = team_model_aliases.get(model) else {
        return model.to_owned();
    };
    let deleted_team_deployment = matches!(
        (view, team_id),
        (Some(router_view), Some(caller_team_id))
            if target.starts_with(&format!("model_name_{caller_team_id}_"))
                && !router_view.has_deployment(target)
    );
    if deleted_team_deployment {
        return model.to_owned();
    }
    target.clone()
}

#[must_use]
pub fn resolve_team_alias(
    model: &RequestedModels,
    team_model_aliases: &BTreeMap<String, String>,
    team_id: Option<&str>,
    view: Option<&dyn ModelAccessView>,
) -> RequestedModels {
    if team_model_aliases.is_empty() {
        return model.clone();
    }
    match model {
        RequestedModels::None => RequestedModels::None,
        RequestedModels::One(name) => RequestedModels::One(live_team_alias_target(
            name,
            team_model_aliases,
            team_id,
            view,
        )),
        RequestedModels::Many(names) => {
            let resolved = names
                .iter()
                .map(|name| live_team_alias_target(name, team_model_aliases, team_id, view))
                .collect();
            RequestedModels::Many(resolved)
        }
    }
}
