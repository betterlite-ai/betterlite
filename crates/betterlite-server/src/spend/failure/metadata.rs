use betterlite_core::api::request_metadata_safety::metadata::{
    metadata_name_from_data,
    // ast-grep-ignore: no-long-names
    metadata_variable_name_for_route,
};
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::{
    UserKeyLogMetadata, logged_api_key,
};
use betterlite_logging::langfuse_projections::steering::truthiness;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use std::mem;
use thiserror::Error;

pub(crate) const GUARDRAIL_INFORMATION: &str = "standard_logging_guardrail_information";
pub(crate) const LITELLM_PARAMS: &str = "litellm_params";
pub(crate) const METADATA: &str = "metadata";
pub(crate) const LITELLM_METADATA: &str = "litellm_metadata";
pub(crate) const MODEL: &str = "model";
pub(crate) const CUSTOM_LLM_PROVIDER: &str = "custom_llm_provider";
pub(crate) const PROXY_SERVER_REQUEST: &str = "proxy_server_request";
pub(crate) const TAGS: &str = "tags";
pub(crate) const USED_CLIENT_OAUTH_TOKEN: &str = "used_client_oauth_token";
pub(crate) const USER_API_KEY: &str = "user_api_key";
pub(crate) const USER_API_KEY_PREFIX: &str = "user_api_key";
pub(crate) const USER_API_KEY_ALIAS: &str = "user_api_key_alias";
pub(crate) const USER_API_KEY_USER_ID: &str = "user_api_key_user_id";
pub(crate) const USER_API_KEY_TEAM_ID: &str = "user_api_key_team_id";
pub(crate) const USER_API_KEY_TEAM_ALIAS: &str = "user_api_key_team_alias";
pub(crate) const USER_API_KEY_ORG_ID: &str = "user_api_key_org_id";
pub(crate) const USER_API_KEY_PROJECT_ID: &str = "user_api_key_project_id";
pub(crate) const STATUS: &str = "status";
pub(crate) const FAILURE_STATUS: &str = "failure";
pub(crate) const ERROR_INFORMATION: &str = "error_information";
pub(crate) const STANDARD_LOGGING_OBJECT: &str = "standard_logging_object";
pub(crate) const LITELLM_TRACE_ID: &str = "litellm_trace_id";
pub(crate) const START_TIME: &str = "start_time";
pub(crate) const COMPLETION_START_TIME: &str = "completion_start_time";
pub(crate) const COMBINED_USAGE_OBJECT: &str = "combined_usage_object";
pub(crate) const RESPONSE_COST: &str = "response_cost";

#[derive(Debug, Error)]
pub enum FailureMetadataError {
    #[error("the key metadata could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// get_sanitized_user_information_from_key plus the failure stamp: the fields B1106 logs for a key request.
pub fn trusted_failure_metadata(
    principal: &PrincipalFacts,
) -> Result<JsonObject, FailureMetadataError> {
    let logged = UserKeyLogMetadata::from(principal);
    let encoded =
        serde_json::to_string(&logged).map_err(|source| FailureMetadataError::Encode { source })?;
    let mut metadata: JsonObject =
        serde_json::from_str(&encoded).map_err(|source| FailureMetadataError::Encode { source })?;
    metadata.insert(
        USER_API_KEY.to_owned(),
        logged_api_key(principal).map_or(Json::Null, |key| Json::Text(key.to_owned())),
    );
    metadata.insert(STATUS.to_owned(), Json::Text(FAILURE_STATUS.to_owned()));
    Ok(metadata)
}

// _proxy_stamped_used_client_oauth_token: only a bool in the proxy's route-selected bucket counts.
#[must_use]
pub fn used_client_oauth_token(request_data: &JsonObject, route: Option<&str>) -> Option<bool> {
    let bucket = match route {
        Some(route) => {
            // ast-grep-ignore: no-long-names
            metadata_variable_name_for_route(route)
        }
        None => metadata_name_from_data(request_data),
    };
    match request_data
        .get(bucket.as_str())
        .and_then(as_object)
        .and_then(|fields| fields.get(USED_CLIENT_OAUTH_TOKEN))
    {
        Some(Json::Bool(flag)) => Some(*flag),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// The bucket the writer's payload reads back: litellm_params.metadata as the merge left it.
#[must_use]
pub(crate) fn persisted_metadata(request_data: &JsonObject) -> Option<&JsonObject> {
    request_data
        .get(LITELLM_PARAMS)
        .and_then(as_object)
        .and_then(|params| params.get(METADATA))
        .and_then(as_object)
}

// request_data.get("metadata") or {}, mutated in place only when a non-empty mapping was already there.
pub(crate) fn merge_failure_metadata(request_data: &mut JsonObject, trusted: JsonObject) {
    let had_metadata =
        matches!(request_data.get(METADATA), Some(Json::Object(fields)) if !fields.is_empty());
    let mut existing = match request_data.get(METADATA) {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => JsonObject::new(),
    };
    existing.extend(trusted);
    copy_guardrail_information(request_data, &mut existing);
    let mut params = take_litellm_params(request_data);
    let tags = params
        .get(METADATA)
        .and_then(as_object)
        .and_then(|fields| fields.get(TAGS))
        .filter(|tags| truthiness(tags))
        .cloned();
    let proxy_request = request_data
        .get(PROXY_SERVER_REQUEST)
        .filter(|value| truthiness(value))
        .cloned()
        .or_else(|| {
            params
                .get(PROXY_SERVER_REQUEST)
                .filter(|value| truthiness(value))
                .cloned()
        })
        .unwrap_or_else(|| Json::Object(JsonObject::new()));
    let model = params.get(MODEL).filter(|value| truthiness(value)).cloned();
    let provider = params
        .get(CUSTOM_LLM_PROVIDER)
        .filter(|value| truthiness(value))
        .cloned();
    if let Some(nested_tags) = tags {
        existing.insert(TAGS.to_owned(), nested_tags);
    }
    params.insert(PROXY_SERVER_REQUEST.to_owned(), proxy_request);
    params.insert(METADATA.to_owned(), Json::Object(existing.clone()));
    request_data.insert(LITELLM_PARAMS.to_owned(), Json::Object(params));
    if !request_data.contains_key(MODEL) {
        request_data.insert(
            MODEL.to_owned(),
            model.unwrap_or_else(|| Json::Text(String::new())),
        );
    }
    if !request_data.contains_key(CUSTOM_LLM_PROVIDER) {
        request_data.insert(
            CUSTOM_LLM_PROVIDER.to_owned(),
            provider.unwrap_or_else(|| Json::Text(String::new())),
        );
    }
    if had_metadata {
        request_data.insert(METADATA.to_owned(), Json::Object(existing));
    }
}

fn copy_guardrail_information(request_data: &JsonObject, existing: &mut JsonObject) {
    if existing.contains_key(GUARDRAIL_INFORMATION) {
        return;
    }
    let source = request_data
        .get(LITELLM_METADATA)
        .and_then(as_object)
        .and_then(|fields| fields.get(GUARDRAIL_INFORMATION))
        .filter(|value| !matches!(value, Json::Null));
    if let Some(value) = source {
        existing.insert(GUARDRAIL_INFORMATION.to_owned(), value.clone());
    }
}

pub(crate) fn take_litellm_params(request_data: &mut JsonObject) -> JsonObject {
    match request_data.get_mut(LITELLM_PARAMS) {
        Some(Json::Object(fields)) => mem::take(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => JsonObject::new(),
    }
}

pub(crate) fn as_object(value: &Json) -> Option<&JsonObject> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn object_mut(value: &mut Json) -> Option<&mut JsonObject> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
