use betterlite_config::sections::request_model_overrides::Settings as ModelOverridesSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::OverridesPrincipal;
use crate::api::request_model_overrides::error::RequestModelOverridesError;

const SERVICE_ACCOUNT_ID_KEY: &str = "service_account_id";
const ENFORCED_PARAMS_KEY: &str = "enforced_params";

// _get_enforced_params: general settings, then the service-account list, then the key's own metadata list
pub(crate) fn resolve_enforced_params(
    settings: &ModelOverridesSettings,
    principal: &OverridesPrincipal,
) -> Result<Option<Vec<String>>, RequestModelOverridesError> {
    let mut enforced = settings.enforced_params.clone();
    // check_if_token_is_service_account: the token's metadata carries service_account_id
    let service_account = principal
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.contains_key(SERVICE_ACCOUNT_ID_KEY));
    if service_account && let Some(extra) = settings.service_account_params.as_ref() {
        enforced
            .get_or_insert_with(Vec::new)
            .extend(extra.iter().cloned());
    }
    if let Some(extra) = key_metadata_params(principal.metadata.as_ref())? {
        enforced.get_or_insert_with(Vec::new).extend(extra);
    }
    Ok(enforced)
}

// the key metadata list is appended last, and a present empty list still turns None into an empty list
fn key_metadata_params(
    metadata: Option<&JsonObject>,
) -> Result<Option<Vec<String>>, RequestModelOverridesError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    match metadata.get(ENFORCED_PARAMS_KEY) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => text_items(items).map(Some),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(params_type_error())
        }
    }
}

fn text_items(items: &[Json]) -> Result<Vec<String>, RequestModelOverridesError> {
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Json::Text(text) => texts.push(text.clone()),
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(params_type_error());
            }
        }
    }
    Ok(texts)
}

fn params_type_error() -> RequestModelOverridesError {
    RequestModelOverridesError::InvalidField {
        field: format!("metadata.{ENFORCED_PARAMS_KEY}"),
        expected: "a list of strings",
    }
}

// _enforced_params_check: None passes, a non-empty list needs a premium user, then every param is required
pub(crate) fn check_enforced_params(
    data: &RequestData,
    enforced: Option<&[String]>,
    premium_user: bool,
) -> Result<(), RequestModelOverridesError> {
    let Some(enforced) = enforced else {
        return Ok(());
    };
    if !enforced.is_empty() && !premium_user {
        return Err(RequestModelOverridesError::NotPremiumUser);
    }
    for param in enforced {
        check_enforced_param(data, param)?;
    }
    Ok(())
}

// a param is a top-level key or one nested key; LiteLLM checks neither branch past two segments
fn check_enforced_param(data: &RequestData, param: &str) -> Result<(), RequestModelOverridesError> {
    let segments: Vec<&str> = param.split('.').collect();
    match segments.as_slice() {
        [field] => {
            if data.contains_key(*field) {
                return Ok(());
            }
            Err(RequestModelOverridesError::MissingEnforcedParam {
                param: (*field).to_owned(),
            })
        }
        [outer, inner] => match data.get(*outer) {
            Some(Json::Object(entry)) if entry.contains_key(*inner) => Ok(()),
            Some(Json::Object(_)) => Err(RequestModelOverridesError::MissingEnforcedParam {
                param: param.to_owned(),
            }),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                Err(RequestModelOverridesError::EnforcedParamParentNotObject {
                    field: (*outer).to_owned(),
                    param: param.to_owned(),
                })
            }
            None => Err(RequestModelOverridesError::MissingEnforcedParam {
                param: (*outer).to_owned(),
            }),
        },
        [] | [_, _, _, ..] => Ok(()),
    }
}
