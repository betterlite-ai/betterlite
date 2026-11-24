use betterlite_auth::request_body_safety::{
    FormMetadataExtractor, ModelGrantLookup, ModelGrants, RequestBodySafety, SafetyError,
    is_request_body_safe,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::debug;

use crate::health_access_and_connections::credential_scope::config_base_for_health_check;
use crate::health_access_and_connections::visibility::{HealthRouter, LookupFailure};

// The configuration the request matched: its connection params and the model_info the auth check reads
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelLookup {
    pub config_params: JsonObject,
    pub model_info: Option<JsonObject>,
}

// The deployment a request matches: an id wins, then the proxy name, then a provider model; a failure leaves its params
#[must_use]
pub fn lookup_deployment(
    router: Option<&dyn HealthRouter>,
    request_params: &JsonObject,
    request_model_info: Option<&JsonObject>,
) -> ModelLookup {
    let Some(lookup_router) = router else {
        return ModelLookup::default();
    };
    let model_name = text_value(request_params, "model");
    let request_model_id = request_model_info.and_then(|info| text_value(info, "id"));
    match route_lookup(lookup_router, model_name, request_model_id) {
        Ok(found) => found,
        Err(error) => {
            debug!(
                model = model_name.unwrap_or("None"),
                error = %error,
                "Could not find model in router. Proceeding with request params only."
            );
            ModelLookup::default()
        }
    }
}

fn route_lookup(
    router: &dyn HealthRouter,
    model_name: Option<&str>,
    request_model_id: Option<&str>,
) -> Result<ModelLookup, LookupFailure> {
    if let Some(requested) = request_model_id
        && let Some(deployment) = router.get_deployment(requested)?
    {
        return Ok(ModelLookup {
            config_params: without_nulls(&deployment.litellm_params),
            model_info: Some(without_nulls(&deployment.model_info)),
        });
    }
    let Some(name) = model_name else {
        return Ok(ModelLookup::default());
    };
    let mut deployments = router.get_model_list(Some(name), None)?;
    if deployments.is_empty() {
        deployments = router
            .get_model_list(None, None)?
            .into_iter()
            .find(|row| text_value(&row.litellm_params, "model") == Some(name))
            .into_iter()
            .collect();
    }
    let Some(first) = deployments.first() else {
        return Ok(ModelLookup::default());
    };
    Ok(ModelLookup {
        config_params: first.litellm_params.clone(),
        model_info: Some(first.model_info.clone()),
    })
}

// pydantic's model_dump(exclude_none=True) on the deployment by id: a top-level null key is absent
fn without_nulls(map: &JsonObject) -> JsonObject {
    map.iter()
        .filter(|(_key, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// {**config_base_for_health_check(...), **request}: the request's own keys win and keep the config's order
#[must_use]
pub fn merge_request_params(
    config_params: &JsonObject,
    request_params: &JsonObject,
    allow_client_side_credentials: bool,
    banned: &[&str],
) -> JsonObject {
    let mut merged = config_base_for_health_check(
        config_params,
        request_params,
        allow_client_side_credentials,
        banned,
    );
    for (key, value) in request_params {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

// B884's request-body check on the request's own params: no outbound URL reaches the probe from a request
pub fn reject_banned_params(
    params: &JsonObject,
    allow_client_side_credentials: bool,
) -> Result<(), SafetyError> {
    let grants = NoGrants;
    let metadata = NoFormMetadata;
    let scan = RequestBodySafety {
        body: params,
        model: "",
        allow_client_side_credentials,
        allowed_hosts: &[],
        grants: &grants,
        metadata: &metadata,
        max_fallbacks: 0_i64,
    };
    is_request_body_safe(&scan)
}

// `mode or litellm_params.pop("mode", None)`: the params key is shifted out only when the body sent no mode
#[must_use]
pub fn take_mode(params: &mut JsonObject) -> Option<String> {
    match params.shift_remove("mode") {
        Some(Json::Text(mode)) => Some(mode),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn text_value<'map>(map: &'map JsonObject, key: &str) -> Option<&'map str> {
    match map.get(key) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[derive(Debug)]
struct NoGrants;

impl ModelGrantLookup for NoGrants {
    fn grants(&self, _model_group: &str) -> ModelGrants<'_> {
        ModelGrants::Absent
    }
}

#[derive(Debug)]
struct NoFormMetadata;

impl FormMetadataExtractor for NoFormMetadata {
    // a parsed JSON object carries no multipart form fields, so no replacement of this constant adapter is observable
    #[cfg_attr(test, mutants::skip)]
    fn top_level_names(&self, _form_data: &JsonObject, _prefix: &str) -> Vec<String> {
        Vec::new()
    }
}
