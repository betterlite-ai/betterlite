use std::collections::BTreeMap;
use std::fmt;

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use super::TextCompletionConfigs;
use super::error::{ErrorSource, TextCompletionError, TextCompletionFailure};
use super::logging::{CompletionLogger, PreCallRecord, SecretHeaders};
use super::sdk::ByteStream;
use super::stream::CompletionStream;

pub struct TextCompletionRequest {
    pub model: Option<String>,
    pub messages: Option<Vec<Value>>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: f64,
    pub organization: Option<String>,
    pub optional_params: Map<String, Value>,
    // None and an empty map are different calls: an empty map keeps its own extra_headers and adds no auth
    pub headers: Option<SecretHeaders>,
    pub acompletion: bool,
}

pub struct TextCompletionReply {
    pub response: Value,
    pub hidden_original_response: Option<String>,
}

pub enum TextCompletionOutcome {
    Buffered(TextCompletionReply),
    Stream(CompletionStream),
}

// What a sent request holds until post_call runs: the SDK dump or the opened byte stream
pub enum SentCompletion {
    Buffered(Value),
    Stream(ByteStream),
}

impl fmt::Debug for SentCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffered(_) => f.write_str("SentCompletion::Buffered"),
            Self::Stream(_) => f.write_str("SentCompletion::Stream"),
        }
    }
}

#[derive(Debug)]
pub struct TextCompletionCall {
    pub(super) context: CallContext,
}

pub(super) struct CallContext {
    pub model: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: f64,
    pub organization: Option<String>,
    pub data: Map<String, Value>,
    pub headers: SecretHeaders,
    pub max_retries: Option<i64>,
    pub acompletion: bool,
    pub streaming: bool,
}

// data carries the caller's headers under extra_headers, so Debug names its keys and no values
impl fmt::Debug for CallContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallContext")
            .field("model", &self.model)
            .field("api_key", &self.api_key)
            .field("api_base", &self.api_base)
            .field("timeout", &self.timeout)
            .field("organization", &self.organization)
            .field("data", &self.data.keys().collect::<Vec<_>>())
            .field("headers", &self.headers)
            .field("max_retries", &self.max_retries)
            .field("acompletion", &self.acompletion)
            .field("streaming", &self.streaming)
            .finish()
    }
}

#[must_use]
pub fn validate_environment(api_key: Option<&SecretString>) -> SecretHeaders {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    SecretHeaders(headers)
}

const MAX_RETRIES_TYPE: &str = "the max_retries parameter must be an integer";

fn pop_max_retries(data: &mut Map<String, Value>) -> Result<Option<i64>, TextCompletionFailure> {
    match data.remove("max_retries") {
        None => Ok(Some(2_i64)),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| TextCompletionFailure::invalid(MAX_RETRIES_TYPE.to_owned())),
        Some(_other) => Err(TextCompletionFailure::invalid(MAX_RETRIES_TYPE.to_owned())),
    }
}

// Truthy caller headers replace extra_headers; None builds the defaults; an empty map is kept as given
fn resolve_headers(request: &mut TextCompletionRequest) -> SecretHeaders {
    let Some(caller) = request.headers.take() else {
        return validate_environment(request.api_key.as_ref());
    };
    if !caller.0.is_empty() {
        let fields = caller
            .0
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())));
        request
            .optional_params
            .insert("extra_headers".to_owned(), Value::Object(fields.collect()));
    }
    caller
}

pub(super) fn prepare(
    configs: &TextCompletionConfigs,
    mut request: TextCompletionRequest,
    provider: ProviderId,
    logger: &mut dyn CompletionLogger,
) -> Result<CallContext, TextCompletionFailure> {
    let streaming = match request.optional_params.get("stream") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_other) => {
            return Err(TextCompletionFailure::invalid(
                "the stream parameter must be a boolean".to_owned(),
            ));
        }
    };
    let headers = resolve_headers(&mut request);
    let (Some(model), Some(messages)) = (request.model, request.messages) else {
        return Err(TextCompletionFailure::Wrapped(
            TextCompletionError::wrapped(ErrorSource::Raised),
        ));
    };
    let transform = configs.selected(provider);
    let mut data = transform(&model, &messages, request.optional_params.clone())
        .map_err(|source| TextCompletionError::wrapped(ErrorSource::Transform(source)))?;
    let max_retries = pop_max_retries(&mut data)?;
    logger
        .pre_call(PreCallRecord {
            input: messages,
            api_key: request.api_key.clone(),
            headers: headers.clone(),
            api_base: request.api_base.clone(),
            complete_input_dict: data.clone(),
        })
        .map_err(|source| TextCompletionError::wrapped(ErrorSource::Logging(source)))?;
    Ok(CallContext {
        model,
        api_key: request.api_key,
        api_base: request.api_base,
        timeout: request.timeout,
        organization: request.organization,
        data,
        headers,
        max_retries,
        acompletion: request.acompletion,
        streaming,
    })
}
