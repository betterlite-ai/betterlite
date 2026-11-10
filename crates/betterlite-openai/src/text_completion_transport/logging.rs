use std::collections::BTreeMap;
use std::fmt;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamOptions};
use betterlite_streaming::stream_chunk_shaping::tokens::check_is_function_call;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

// LiteLLM logs a copy of the live headers; neither Debug nor an error may print their values
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretHeaders(pub BTreeMap<String, String>);

impl SecretHeaders {
    #[must_use]
    pub fn from_response(headers: &HeaderMap) -> Self {
        Self(
            headers
                .iter()
                .map(|(name, value)| {
                    // HTTP/1 header bytes decode as Latin-1 where they are not ASCII
                    let text = value.to_str().map_or_else(
                        |_source| {
                            value
                                .as_bytes()
                                .iter()
                                .map(|byte| char::from(*byte))
                                .collect()
                        },
                        str::to_owned,
                    );
                    (name.as_str().to_owned(), text)
                })
                .collect(),
        )
    }
}

impl fmt::Debug for SecretHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|name| (name, "<redacted>")))
            .finish()
    }
}

// The handler hands post_call the SDK object for the async path and its dict for the sync path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    DumpedDict,
    SdkObject,
}

#[derive(Debug)]
pub struct PreCallRecord {
    pub input: Vec<Value>,
    pub api_key: Option<SecretString>,
    pub headers: SecretHeaders,
    pub api_base: Option<String>,
    pub complete_input_dict: serde_json::Map<String, Value>,
}

#[derive(Debug)]
pub struct PostCallRecord {
    pub api_key: Option<SecretString>,
    pub original_response: Value,
    pub kind: ResponseKind,
    pub headers: SecretHeaders,
    pub api_base: Option<String>,
}

// What CustomStreamWrapper reads from the logging object besides the two call hooks
pub struct StreamDefaults {
    pub stream_options: Option<StreamOptions>,
    pub merge_reasoning_in_choices: bool,
    pub is_function_call: bool,
    pub base_hidden_params: JsonObject,
}

pub trait CompletionLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    fn post_call(&mut self, record: PostCallRecord);

    fn stream_defaults(&self) -> StreamDefaults;
}

// B249's pre-call and post-call dispatch over the transport's records
pub struct B249Logger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

fn decision(value: Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(flag),
        Value::Number(number) => Json::Number(number),
        Value::String(text) => Json::Text(text),
        Value::Array(items) => Json::List(items.into_iter().map(decision).collect()),
        Value::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, entry)| (name, decision(entry)))
                .collect(),
        ),
    }
}

fn decision_map(fields: serde_json::Map<String, Value>) -> IndexMap<String, Json> {
    fields
        .into_iter()
        .map(|(name, value)| (name, decision(value)))
        .collect()
}

fn additional_args(
    api_base: Option<String>,
    headers: SecretHeaders,
    body: serde_json::Map<String, Value>,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers: Some(
            headers
                .0
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect(),
        ),
        complete_input_dict: Some(RawBody::Object(decision_map(body))),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

fn shaping_json(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(shaping_json).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), shaping_json(entry)))
                .collect(),
        ),
    }
}

fn stream_options(value: &Json) -> StreamOptions {
    let include_usage = matches!(
        value,
        Json::Object(fields) if matches!(fields.get("include_usage"), Some(Json::Bool(true)))
    );
    StreamOptions { include_usage }
}

fn base_hidden_params(litellm_params: &IndexMap<String, Json>) -> JsonObject {
    let mut hidden = JsonObject::new();
    hidden.insert("model_id".to_owned(), JsonValue::Null);
    hidden.insert(
        "api_base".to_owned(),
        litellm_params
            .get("api_base")
            .map_or(JsonValue::Null, shaping_json),
    );
    hidden.insert(
        "additional_headers".to_owned(),
        JsonValue::Object(JsonObject::new()),
    );
    hidden.insert("response_cost".to_owned(), JsonValue::Null);
    hidden
}

impl CompletionLogger for B249Logger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let call = PreCall {
            input: Json::List(record.input.into_iter().map(decision).collect()),
            model: None,
            additional_args: additional_args(
                record.api_base,
                record.headers,
                record.complete_input_dict,
            ),
        };
        pre_call(self.state, &call, self.env)?;
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        let call = PostCall {
            original_response: decision(record.original_response),
            input: Json::Null,
            additional_args: additional_args(
                record.api_base,
                record.headers,
                serde_json::Map::new(),
            ),
        };
        post_call(self.state, &call, self.env);
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: self.state.stream_options.as_ref().map(stream_options),
            merge_reasoning_in_choices: matches!(
                self.state
                    .litellm_params
                    .get("merge_reasoning_content_in_choices"),
                Some(Json::Bool(true))
            ),
            is_function_call: check_is_function_call(Some(&JsonObject::from_iter(
                self.state
                    .optional_params
                    .iter()
                    .map(|(name, value)| (name.clone(), shaping_json(value))),
            ))),
            base_hidden_params: base_hidden_params(&self.state.litellm_params),
        }
    }
}
