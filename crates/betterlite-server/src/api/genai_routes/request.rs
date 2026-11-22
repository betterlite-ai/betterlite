#![expect(
    clippy::disallowed_types,
    reason = "B1100's decoder hands back untyped JSON; this file converts it once into the typed Json the GenAI pipeline carries"
)]

use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{DecodedJson, JsonObject, RequestBodyError};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::request_body::decode_request_body;

const MODEL: &str = "model";
const CONTENTS: &str = "contents";
const STREAM: &str = "stream";
// the google-genai SDK's (?alt=sse) stream must not receive OpenAI's data: [DONE] terminator
const SKIP_STREAM_DONE: &str = "_litellm_skip_openai_stream_done";
const RAW_SSE_STREAM: &str = "_litellm_raw_sse_stream";

// _read_request_body: an empty body is {}, a body the repair cannot decode is {}, anything else must be an object
pub(super) fn decode(body: &[u8]) -> Result<Object, RequestBodyError> {
    match decode_request_body(body, &ApiLimits::default()) {
        Ok(DecodedJson::Parsed { body: parsed, .. }) => Ok(typed(parsed)),
        Ok(DecodedJson::Unreadable(error)) => {
            // LiteLLM's own catch-all turns a body it cannot decode as UTF-8 into {}
            tracing::error!(%error, "unexpected error reading the GenAI request body, using an empty object");
            Ok(Object::new())
        }
        Err(error) => Err(error),
    }
}

// data["model"] = model_name only runs when the body has no model key, so an explicit null survives
pub(super) fn with_model(data: &mut Object, model: &str) {
    if data.contains_key(MODEL) {
        return;
    }
    data.insert(MODEL.to_owned(), Json::Text(model.to_owned()));
}

// data["stream"] = True and the two raw-SSE flags, assigned over whatever the caller sent
pub(super) fn with_stream_flags(data: &mut Object) {
    data.insert(STREAM.to_owned(), Json::Bool(true));
    data.insert(SKIP_STREAM_DONE.to_owned(), Json::Bool(true));
    data.insert(RAW_SSE_STREAM.to_owned(), Json::Bool(true));
}

// data.get("contents", []): absent or null take the empty list, a list is used as given
pub(super) fn contents(data: &Object) -> Result<Json, Box<ProxyException>> {
    match data.get(CONTENTS) {
        None | Some(Json::Null) => Ok(Json::List(Vec::new())),
        Some(Json::List(items)) => Ok(Json::List(items.clone())),
        // TokenCountRequest.contents is list[dict] | None, so any other JSON type is a wrong type
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(Box::new(ProxyException::new(ProxyExceptionInit {
                message: format!("'{CONTENTS}' has the wrong type"),
                error_type: "invalid_request_error".to_owned(),
                param: Some(CONTENTS.to_owned()),
                code: ExceptionCode::Status(400),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            })))
        }
    }
}

fn typed(body: JsonObject) -> Object {
    body.into_iter()
        .map(|(key, value)| (key, json(value)))
        .collect()
}

fn json(value: serde_json::Value) -> Json {
    match value {
        serde_json::Value::Null => Json::Null,
        serde_json::Value::Bool(boolean) => Json::Bool(boolean),
        serde_json::Value::Number(number) => Json::Number(number),
        serde_json::Value::String(text) => Json::Text(text),
        serde_json::Value::Array(items) => Json::List(items.into_iter().map(json).collect()),
        serde_json::Value::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(field, nested)| (field, json(nested)))
                .collect(),
        ),
    }
}
