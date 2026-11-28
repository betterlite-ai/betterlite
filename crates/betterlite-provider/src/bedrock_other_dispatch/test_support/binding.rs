use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::transcription::binding::{BindingArguments, TranscriptionBinding};
use betterlite_bedrock::transcription::error::{CallError, TranscriptionError};
use betterlite_bedrock::transcription::params::Params;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Number;

// The native binding's keyword arguments, captured at the seam LiteLLM reaches with NO_PYTHON
#[derive(Clone, Default)]
pub struct RecordedArguments(Arc<Mutex<Vec<Json>>>);

impl RecordedArguments {
    #[must_use]
    pub fn values(&self) -> Vec<Json> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, value: Json) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }
}

// Stands in for the native call: it records the arguments it was handed and answers a fixed text
#[derive(Clone)]
pub struct Recording {
    pub text: String,
    pub failure: Option<u16>,
    pub seen: RecordedArguments,
}

impl Recording {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            failure: None,
            seen: RecordedArguments::default(),
        }
    }

    #[must_use]
    pub fn failing(status: u16) -> Self {
        Self {
            text: String::new(),
            failure: Some(status),
            seen: RecordedArguments::default(),
        }
    }

    fn record(&self, arguments: &BindingArguments<'_>) -> Result<Params, CallError> {
        self.seen.push(arguments_value(arguments)?);
        match self.failure {
            Some(status) => Err(CallError::Status {
                status,
                body: String::from("the fixture binding failed"),
            }),
            None => Ok(Params::from_iter([(
                "text".to_owned(),
                Json::Text(self.text.clone()),
            )])),
        }
    }
}

#[async_trait]
impl TranscriptionBinding for Recording {
    type Error = CallError;

    fn transcribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, CallError> {
        self.record(arguments)
    }

    async fn atranscribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, CallError> {
        self.record(arguments)
    }
}

// A JSON value that fails here would be a fixture bug, and the encoding error names it
#[must_use]
pub fn arguments_value(arguments: &BindingArguments<'_>) -> Result<Json, CallError> {
    let call = arguments.call;
    let audio = encode(&arguments.audio)?;
    let extra_headers = match call.extra_headers {
        Some(headers) => encode(headers)?,
        None => Json::Null,
    };
    let optional_params = encode(call.optional_params)?;
    let timeout = arguments
        .timeout_seconds
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number);
    Ok(Json::Object(IndexMap::from([
        (String::from("model"), Json::Text(call.model.to_owned())),
        (String::from("audio"), audio),
        (String::from("api_key"), optional_secret(call.api_key)),
        (
            String::from("api_base"),
            call.api_base
                .map_or(Json::Null, |base| Json::Text(base.to_owned())),
        ),
        (
            String::from("custom_llm_provider"),
            Json::Text(call.custom_llm_provider.to_owned()),
        ),
        (String::from("extra_headers"), extra_headers),
        (String::from("optional_params"), optional_params),
        (String::from("timeout_seconds"), timeout),
    ])))
}

fn optional_secret(secret: Option<&SecretString>) -> Json {
    secret.map_or(Json::Null, |value| {
        Json::Text(value.expose_secret().to_owned())
    })
}

fn encode<T>(value: &T) -> Result<Json, CallError>
where
    T: serde::Serialize,
{
    let text = serde_json::to_string(value).map_err(encode_failure)?;
    serde_json::from_str(&text).map_err(encode_failure)
}

fn encode_failure(source: serde_json::Error) -> CallError {
    CallError::Transcription(TranscriptionError::Encode(source))
}
