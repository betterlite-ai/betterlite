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

pub struct PreCallRecord {
    pub input: Vec<Value>,
    pub api_key: Option<SecretString>,
    pub headers: SecretHeaders,
    pub api_base: Option<String>,
    pub complete_input_dict: serde_json::Map<String, Value>,
}

// complete_input_dict holds the caller's headers under extra_headers, so Debug names its keys and no values
impl fmt::Debug for PreCallRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreCallRecord")
            .field("input", &self.input)
            .field("api_key", &self.api_key)
            .field("headers", &self.headers)
            .field("api_base", &self.api_base)
            .field(
                "complete_input_dict",
                &self.complete_input_dict.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
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

// The logging crate's pre-call and post-call dispatch over the transport's records
pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// Json and serde_json::Value hold the same shapes, so serde moves values between them without failing
fn additional_args(
    api_base: Option<String>,
    headers: SecretHeaders,
    body: Option<serde_json::Map<String, Value>>,
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
        complete_input_dict: body.map(|fields| {
            RawBody::Object(serde_json::from_value(Value::Object(fields)).unwrap_or_default())
        }),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

fn shaping_json(value: &Json) -> Option<JsonValue> {
    match serde_json::to_value(value) {
        Ok(plain) => serde_json::from_value(plain).ok(),
        Err(_source) => None,
    }
}

fn stream_options(value: &Json) -> StreamOptions {
    let include_usage = matches!(
        value,
        Json::Object(fields) if matches!(fields.get("include_usage"), Some(Json::Bool(true)))
    );
    StreamOptions { include_usage }
}

pub(super) fn base_hidden_params(litellm_params: &IndexMap<String, Json>) -> JsonObject {
    let mut hidden = JsonObject::new();
    let model_id = match litellm_params.get("model_info") {
        Some(Json::Object(info)) => info.get("id").and_then(shaping_json),
        _ => None,
    };
    hidden.insert("model_id".to_owned(), model_id.unwrap_or(JsonValue::Null));
    hidden.insert(
        "api_base".to_owned(),
        litellm_params
            .get("api_base")
            .and_then(shaping_json)
            .unwrap_or(JsonValue::Null),
    );
    hidden.insert(
        "additional_headers".to_owned(),
        JsonValue::Object(JsonObject::new()),
    );
    hidden.insert("response_cost".to_owned(), JsonValue::Null);
    hidden
}

impl CompletionLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let call = PreCall {
            input: serde_json::from_value(Value::Array(record.input)).unwrap_or(Json::Null),
            model: None,
            additional_args: additional_args(
                record.api_base,
                record.headers,
                Some(record.complete_input_dict),
            ),
        };
        pre_call(self.state, &call, self.env)?;
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        let call = PostCall {
            original_response: serde_json::from_value(record.original_response)
                .unwrap_or(Json::Null),
            input: Json::Null,
            additional_args: additional_args(record.api_base, record.headers, None),
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
                    .filter_map(|(name, value)| Some((name.clone(), shaping_json(value)?))),
            ))),
            base_hidden_params: base_hidden_params(&self.state.litellm_params),
        }
    }
}
