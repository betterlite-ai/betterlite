use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::url::QueryParams;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::realtime_connect::QUERY_MODEL_KEY;

use super::{RealtimeProbeError, RealtimeProbeRequest};

pub const LITELLM_CREDENTIAL_NAME_KEY: &str = "litellm_credential_name";
pub const API_KEY_KEY: &str = "api_key";
pub const API_BASE_KEY: &str = "api_base";
pub const API_VERSION_KEY: &str = "api_version";

// the named arguments with the merged parameters underneath them, as the health check resolves them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProbeParameters {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
}

// _model_params_with_stored_credentials: stored values sit underneath the explicit parameters, so an explicit None erases one
#[must_use]
pub fn model_params_with_stored_credentials(
    model_params: &JsonObject,
    stored: &CredentialSnapshot,
) -> JsonObject {
    let credential_name = match model_params.get(LITELLM_CREDENTIAL_NAME_KEY) {
        Some(Json::Text(name)) => Some(name.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let mut merged = credential_name.map_or_else(JsonObject::new, |name| stored.values(name));
    merged.extend(
        model_params
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    merged
}

// `named or merged`, with the empty string the Python `or` treats as absent
#[must_use]
pub fn resolve_probe_parameters(
    request: &RealtimeProbeRequest,
    merged: &JsonObject,
) -> Result<ResolvedProbeParameters, RealtimeProbeError> {
    Ok(ResolvedProbeParameters {
        api_key: resolved_secret(request.api_key.as_ref(), merged, API_KEY_KEY)?,
        api_base: resolved_text(request.api_base.as_deref(), merged, API_BASE_KEY)?,
        api_version: resolved_text(request.api_version.as_deref(), merged, API_VERSION_KEY)?,
    })
}

#[must_use]
pub fn model_query(model: &str) -> QueryParams {
    QueryParams::from([(String::from(QUERY_MODEL_KEY), Some(model.to_owned()))])
}

// _realtime_health_check_auth_headers outside Azure: an absent key sends nothing, an empty one still sends "Bearer "
#[must_use]
pub fn bearer_headers(api_key: Option<&SecretString>) -> Headers {
    match api_key {
        None => Headers::new(),
        Some(key) => Headers::from([(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        )]),
    }
}

// `named or merged`: the named argument wins only when it is a non-empty string
fn resolved_secret(
    named: Option<&SecretString>,
    merged: &JsonObject,
    field: &'static str,
) -> Result<Option<SecretString>, RealtimeProbeError> {
    if let Some(value) = named.filter(|value| !value.expose_secret().is_empty()) {
        return Ok(Some(value.clone()));
    }
    resolved_text(None, merged, field).map(|value| value.map(SecretString::from))
}

fn resolved_text(
    named: Option<&str>,
    merged: &JsonObject,
    field: &'static str,
) -> Result<Option<String>, RealtimeProbeError> {
    if let Some(value) = named.filter(|value| !value.is_empty()) {
        return Ok(Some(value.to_owned()));
    }
    match merged.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RealtimeProbeError::ParameterType { field })
        }
    }
}

#[must_use]
pub fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|number| number != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
