use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_catalog::types::Candidates;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::static_eligibility::environment::active_in_environment;

use super::{API_KEY, Entry, Input, Lookup};
use crate::router_probes::error::OperationError;

pub(super) fn model_info(entry: &Entry) -> JsonObject {
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(entry.id.clone()));
    if let Some(base_model) = &entry.base_model {
        info.insert("base_model".to_owned(), Json::Text(base_model.clone()));
    }
    if let Some(maximum) = entry.max_input_tokens {
        info.insert(
            "max_input_tokens".to_owned(),
            Json::Number(serde_json::Number::from(maximum)),
        );
    }
    if let Some(blocked) = entry.blocked {
        info.insert("blocked".to_owned(), Json::Bool(blocked));
    }
    if let Some(supports) = entry.supports_web_search {
        info.insert("supports_web_search".to_owned(), Json::Bool(supports));
    }
    if let Some(environments) = &entry.supported_environments {
        info.insert(
            "supported_environments".to_owned(),
            Json::List(
                environments
                    .iter()
                    .map(|name| Json::Text(name.clone()))
                    .collect(),
            ),
        );
    }
    info
}

// One registered deployment; None when Router would have skipped it at construction because of its environment.
fn spec(input: &Input, entry: &Entry) -> Result<Option<DeploymentSpec>, OperationError> {
    let info = model_info(entry);
    let active = active_in_environment(&info, Some(&input.environment)).map_err(|error| {
        OperationError::InvalidRequest {
            message: error.to_string(),
        }
    })?;
    if !active {
        return Ok(None);
    }
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(entry.model.clone()));
    params.insert("api_key".to_owned(), Json::Text(API_KEY.to_owned()));
    if let Some(order) = entry.order {
        params.insert(
            "order".to_owned(),
            Json::Number(serde_json::Number::from(order)),
        );
    }
    DeploymentSpec::new(
        entry.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .map(Some)
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn deployable<'entries, I>(input: &Input, entries: I) -> Result<Vec<DeploymentSpec>, OperationError>
where
    I: Iterator<Item = &'entries Entry>,
{
    let mut pool = Vec::new();
    for entry in entries {
        if let Some(built) = spec(input, entry)? {
            pool.push(built);
        }
    }
    Ok(pool)
}

// _common_checks_available_deployment as far as the probe cases reach: a group name, a deployment id, or a litellm model.
pub(super) fn resolve(
    input: &Input,
    lookup: &Lookup,
) -> Result<Option<Candidates>, OperationError> {
    if lookup.specific_deployment {
        let pool = deployable(
            input,
            input
                .models
                .iter()
                .filter(|entry| entry.model == lookup.model),
        )?;
        return Ok(Some(Candidates::Pool(pool)));
    }
    let named: Vec<&Entry> = input
        .models
        .iter()
        .filter(|entry| entry.model_name == lookup.model)
        .collect();
    if !named.is_empty() {
        return Ok(Some(Candidates::Pool(deployable(
            input,
            named.into_iter(),
        )?)));
    }
    let Some(entry) = input.models.iter().find(|entry| entry.id == lookup.model) else {
        return Ok(None);
    };
    Ok(spec(input, entry)?.map(|built| Candidates::Single(Box::new(built))))
}
