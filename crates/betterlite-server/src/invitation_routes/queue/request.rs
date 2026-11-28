use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::http::uri::Authority;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_core::api::request_principal_metadata::principal::strip_callback_config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::{Deserialize, Serialize};

use super::scheduled::QueueError;
use crate::invitation_routes::state::QueueRouteState;
use crate::pre_call::rewrites::apply_overrides;

const STRIP_STREAM_USAGE: &str = "_litellm_strip_stream_usage";
const HOST: &str = "host";
const AUTHORIZATION: &str = "authorization";

#[derive(Clone, Copy)]
pub(super) struct QueueRequest<'call> {
    pub(super) queue: &'call QueueRouteState,
    pub(super) caller: &'call Principal,
    pub(super) parts: &'call Parts,
    pub(super) model: Option<&'call str>,
}

// the fields of UserAPIKeyAuth this route copies into the request data, spelled as LiteLLM serializes them
#[derive(Default, Deserialize)]
#[serde(default)]
struct KeyView {
    api_key: Option<String>,
    token: Option<String>,
    is_session_token: bool,
    key_alias: Option<String>,
    user_id: Option<String>,
    team_id: Option<String>,
    object_permission_id: Option<String>,
    team_object_permission_id: Option<String>,
    metadata: JsonValue,
}

pub(super) fn build_request(
    request: QueueRequest<'_>,
    body: &[u8],
    data: &mut JsonMap,
) -> Result<(), QueueError> {
    let mut decoded: JsonMap =
        serde_json::from_slice(body).map_err(|error| QueueError::Message(error.to_string()))?;
    decoded.remove(STRIP_STREAM_USAGE);
    let url = request_url(request.parts);
    let original = decoded.clone();
    let proxy_server = proxy_server_request(&url, request.parts, &original);
    decoded.insert("proxy_server_request".to_owned(), proxy_server);
    let model = resolved_model(request, &decoded);
    decoded.insert(
        "model".to_owned(),
        model.map_or(JsonValue::Null, JsonValue::String),
    );
    let key = key_view(request.caller);
    if decoded.get("user").is_none_or(JsonValue::is_null)
        && let Some(user_id) = queue_user_id(request.caller, &key)
    {
        decoded.insert("user".to_owned(), JsonValue::String(user_id.to_owned()));
    }
    write_metadata(&mut decoded, &key, request.parts, &url);
    apply_overrides(&mut decoded, &request.queue.overrides);
    *data = decoded;
    Ok(())
}

// the generator hands its request data back as the decision tree, and the failure hook reads a JSON map
pub(super) fn data_map(object: &JsonObject) -> JsonMap {
    let mut map = JsonMap::new();
    for (key, value) in object {
        map.insert(key.clone(), json_value(value));
    }
    map
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(list) => JsonValue::Array(list.iter().map(json_value).collect()),
        Json::Object(object) => JsonValue::Object(data_map(object)),
    }
}

// general_settings.completion_model, the CLI model, the query parameter, then the request body
fn resolved_model(request: QueueRequest<'_>, data: &JsonMap) -> Option<String> {
    request
        .queue
        .completion_model
        .clone()
        .or_else(|| request.queue.cli_model.clone())
        .or_else(|| request.model.map(str::to_owned))
        .or_else(|| {
            data.get("model")
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        })
}

// LiteLLM fills user from user_api_key_dict.user_id, and a principal authenticated without a key keeps its own id
fn queue_user_id<'call>(caller: &'call Principal, key: &'call KeyView) -> Option<&'call str> {
    key.user_id
        .as_deref()
        .or_else(|| caller.user.as_ref().map(|user| user.id.as_str()))
}

fn key_view(caller: &Principal) -> KeyView {
    let Some(key) = caller.source_key.0.as_ref() else {
        return KeyView::default();
    };
    match serde_json::to_value(key) {
        Ok(value) => serde_json::from_value(value).unwrap_or_default(),
        Err(_error) => KeyView::default(),
    }
}

// Router.schedule_acompletion receives all of this as keyword arguments, so every slot is written in LiteLLM's order
#[derive(Serialize)]
struct KeyArgs<'key> {
    user_api_key: Option<&'key str>,
    user_api_key_hash: Option<&'key str>,
    user_api_key_metadata: JsonValue,
    headers: JsonMap,
    user_api_key_alias: Option<&'key str>,
    user_api_key_user_id: Option<&'key str>,
    user_api_key_team_id: Option<&'key str>,
    #[serde(rename = "user_api_key_object_permission_id")]
    key_object_permission_id: Option<&'key str>,
    #[serde(rename = "user_api_key_team_object_permission_id")]
    team_object_permission_id: Option<&'key str>,
    endpoint: &'key str,
}

fn write_metadata(data: &mut JsonMap, key: &KeyView, parts: &Parts, endpoint: &str) {
    let mut bucket = match data.remove("metadata") {
        Some(JsonValue::Object(bucket)) => bucket,
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_),
        )
        | None => JsonMap::new(),
    };
    let mut headers = headers_object(&parts.headers);
    headers.remove(AUTHORIZATION);
    let logged = logged_api_key(key);
    let entry = KeyArgs {
        user_api_key: logged,
        user_api_key_hash: logged,
        user_api_key_metadata: stripped_key_metadata(&key.metadata),
        headers,
        user_api_key_alias: key.key_alias.as_deref(),
        user_api_key_user_id: key.user_id.as_deref(),
        user_api_key_team_id: key.team_id.as_deref(),
        key_object_permission_id: key.object_permission_id.as_deref(),
        team_object_permission_id: key.team_object_permission_id.as_deref(),
        endpoint,
    };
    let JsonValue::Object(fields) = serde_json::to_value(entry).unwrap_or_default() else {
        return;
    };
    bucket.extend(fields);
    data.insert("metadata".to_owned(), JsonValue::Object(bucket));
}

// LiteLLMProxyRequestSetup.get_logged_api_key: auth stores the key's hash under api_key, and a session token logs as its alias
fn logged_api_key(key: &KeyView) -> Option<&str> {
    if key.is_session_token
        && let Some(alias) = key.key_alias.as_deref()
    {
        return Some(alias);
    }
    key.api_key.as_deref().or(key.token.as_deref())
}

fn stripped_key_metadata(value: &JsonValue) -> JsonValue {
    let Ok(metadata) = serde_json::from_value::<Metadata>(value.clone()) else {
        return JsonValue::Object(JsonMap::new());
    };
    let stripped = strip_callback_config(&metadata);
    serde_json::to_value(&stripped).unwrap_or_else(|_error| JsonValue::Object(JsonMap::new()))
}

fn proxy_server_request(url: &str, parts: &Parts, body: &JsonMap) -> JsonValue {
    let mut request = JsonMap::new();
    request.insert("url".to_owned(), JsonValue::String(url.to_owned()));
    request.insert(
        "method".to_owned(),
        JsonValue::String(parts.method.as_str().to_owned()),
    );
    request.insert(
        "headers".to_owned(),
        JsonValue::Object(headers_object(&parts.headers)),
    );
    request.insert("body".to_owned(), JsonValue::Object(body.clone()));
    JsonValue::Object(request)
}

// Starlette's request.headers: lowercased names, and a name sent twice keeps both values
fn headers_object(headers: &HeaderMap) -> JsonMap {
    let mut object = JsonMap::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        let merged = object
            .get(name.as_str())
            .and_then(JsonValue::as_str)
            .map_or_else(|| text.to_owned(), |existing| format!("{existing}, {text}"));
        object.insert(name.as_str().to_owned(), JsonValue::String(merged));
    }
    object
}

fn request_url(parts: &Parts) -> String {
    let scheme = parts.uri.scheme_str().unwrap_or("http");
    let authority = parts
        .uri
        .authority()
        .map(Authority::as_str)
        .or_else(|| {
            let host = parts.headers.get(HOST)?;
            host.to_str().ok()
        })
        .unwrap_or_default();
    let path = parts
        .uri
        .path_and_query()
        .map_or("", |path_and_query| path_and_query.as_str());
    if authority.is_empty() {
        return path.to_owned();
    }
    format!("{scheme}://{authority}{path}")
}
