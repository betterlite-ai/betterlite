use std::collections::BTreeMap;

use betterlite_auth::auth_model_grants::source_adapter::token_in_metadata;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_metadata_safety::metadata::metadata_name_from_data;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_router::router_predicates::JsonMap;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;

use crate::routing::cache_alternatives::error::CacheAlternativesError;

const PROXY_REQUEST: &str = "proxy_server_request";
const AUTH_FIELD: &str = "user_api_key_auth";
const CONFIG_FIELD: &str = "config";
// request_kwargs entries that reroute or rewrite the call after the comparison, so it must not run
const REFUSED_REQUEST_FIELDS: [&str; 9] = [
    "guardrails",
    "cache_control_injection_points",
    "api_key",
    "api_base",
    "extra_headers",
    "prompt_id",
    "mock_response",
    "model_info",
    "custom_llm_provider",
];
const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";

// _ProxyRequest: the strict proxy call a prediction rebuilds its body from.
pub(crate) struct ProxyRequest {
    pub(crate) url: String,
    pub(crate) body: JsonMap,
    pub(crate) headers: HeaderMap,
}

impl ProxyRequest {
    pub(crate) fn from_kwargs(request_kwargs: &JsonMap) -> Option<Self> {
        let Some(Json::Object(fields)) = request_kwargs.get(PROXY_REQUEST) else {
            return None;
        };
        let url = text(fields.get("url"))?.to_owned();
        let body = object(fields.get("body"))?.clone();
        let headers = header_map(object(fields.get("headers"))?)?;
        Some(Self { url, body, headers })
    }
}

// _CallerSettings over the metadata bucket: the caller entry must decode, and a non-empty per-key config refuses it
pub(crate) fn caller(
    request_kwargs: &JsonMap,
) -> Result<Option<UserAPIKeyAuth>, CacheAlternativesError> {
    let field = metadata_name_from_data(request_kwargs);
    let metadata = match request_kwargs.get(field.as_str()) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(entries)) => entries,
        Some(value) if set_to_a_value(value) => {
            return Err(CacheAlternativesError::Metadata {
                field: field.as_str(),
            });
        }
        Some(_) => return Ok(None),
    };
    // an entry that does not decode is not the UserAPIKeyAuth instance LiteLLM requires
    let Some(Json::Object(caller_fields)) = metadata.get(AUTH_FIELD) else {
        return Ok(None);
    };
    let decoded: BTreeMap<String, JsonData> =
        match serde_json::to_value(metadata).and_then(serde_json::from_value) {
            Ok(fields) => fields,
            Err(_) => return Ok(None),
        };
    let Some(token) = token_in_metadata(&JsonData::Object(decoded)) else {
        return Ok(None);
    };
    // _CallerSettings.config: a missing or empty mapping passes and every other config refuses the caller,
    // the non-mapping types having already failed to decode into the UserAPIKeyAuth LiteLLM requires
    match caller_fields.get(CONFIG_FIELD) {
        None | Some(Json::Null) => Ok(Some(token)),
        Some(Json::Object(config)) if config.is_empty() => Ok(Some(token)),
        Some(Json::Object(_) | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Ok(None)
        }
    }
}

// the request fields LiteLLM tests with `any(request_kwargs.get(key) ...)`, so an empty list or object passes
pub(crate) fn refused_request_fields(request_kwargs: &JsonMap) -> bool {
    REFUSED_REQUEST_FIELDS
        .iter()
        .any(|field| request_kwargs.get(*field).is_some_and(set_to_a_value))
}

// whether a value is set at all: LiteLLM refuses the list with any(...), so an empty list or object passes
pub(crate) fn set_to_a_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

const fn object(value: Option<&Json>) -> Option<&JsonMap> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

const fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(found)) => Some(found.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// a value the wire cannot carry keeps its header out of the map, unless that header alone refuses the prediction
fn header_map(fields: &JsonMap) -> Option<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        let Json::Text(header_value) = value else {
            return None;
        };
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        match HeaderValue::from_str(header_value) {
            Ok(parsed) => {
                headers.insert(header_name, parsed);
            }
            Err(_) if prediction_header(header_name.as_str()) => return None,
            Err(_) => {}
        }
    }
    Some(headers)
}

const fn prediction_header(name: &str) -> bool {
    name.eq_ignore_ascii_case(ANTHROPIC_BETA_HEADER)
        || name.eq_ignore_ascii_case(ANTHROPIC_VERSION_HEADER)
}
