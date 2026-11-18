use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::NaiveDate;

use super::catalog::DeprecationCatalog;
use super::dates::parse_json_date;

pub struct ResolvedDeprecation {
    pub deprecation_date: NaiveDate,
    pub litellm_model: Option<String>,
    pub litellm_provider: Option<String>,
}

pub struct Deployment<'deployment> {
    pub model_name: Option<&'deployment str>,
    pub model_info: Option<&'deployment JsonObject>,
    pub litellm_params: Option<&'deployment JsonObject>,
    pub override_date: Option<NaiveDate>,
}

impl<'deployment> Deployment<'deployment> {
    #[must_use]
    pub fn from_spec(spec: &'deployment DeploymentSpec) -> Self {
        Self {
            model_name: Some(spec.model_name.as_str()),
            model_info: Some(&spec.model_info),
            litellm_params: Some(&spec.litellm_params),
            override_date: parse_json_date(spec.model_info.get("deprecation_date")),
        }
    }
}

#[must_use]
pub fn resolve_deprecation(
    deployment: &Deployment<'_>,
    catalog: &dyn DeprecationCatalog,
) -> Option<ResolvedDeprecation> {
    let raw_model = text_field(deployment.litellm_params, "model");
    if let Some(date) = deployment.override_date {
        return Some(ResolvedDeprecation {
            deprecation_date: date,
            litellm_model: raw_model.map(str::to_owned),
            litellm_provider: text_field(deployment.model_info, "litellm_provider")
                .map(str::to_owned),
        });
    }
    let unprefixed = raw_model
        .and_then(|model| model.split_once('/'))
        .map(|(_, suffix)| suffix);
    [
        text_field(deployment.model_info, "base_model"),
        raw_model,
        unprefixed,
    ]
    .into_iter()
    .flatten()
    .find_map(|key| cost_map_lookup(catalog, key))
}

fn cost_map_lookup(catalog: &dyn DeprecationCatalog, key: &str) -> Option<ResolvedDeprecation> {
    if key.is_empty() {
        return None;
    }
    let entry = catalog.deprecation(key)?;
    Some(ResolvedDeprecation {
        deprecation_date: entry.deprecation_date,
        litellm_model: Some(key.to_owned()),
        litellm_provider: entry.litellm_provider,
    })
}

fn text_field<'fields>(fields: Option<&'fields JsonObject>, key: &str) -> Option<&'fields str> {
    match fields.and_then(|fields| fields.get(key)) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
