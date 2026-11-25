use std::collections::BTreeMap;

use betterlite_core::api::file_content_stream::{ContentCall, ContentKwargs};
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_models::controls::json_value::JsonValue;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod tests;

const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024;
const READ_TIMEOUT_SHAPES: &str =
    "seconds, a number written as text, or the phases of an httpx.Timeout";
const REQUEST_TIMEOUT_SHAPES: &str = "seconds or a number written as text";

const fn default_chunk_size() -> u64 {
    DEFAULT_CHUNK_SIZE
}

// Everything one file_content call carries, in the names the Python shim sends them under.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RequestWire {
    pub(crate) file_id: String,
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<BTreeMap<String, String>>,
    #[serde(default = "default_chunk_size")]
    pub(crate) chunk_size: u64,
    pub(crate) stream: bool,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) timeout: Option<JsonValue>,
    pub(crate) request_timeout: Option<JsonValue>,
    pub(crate) max_retries: Option<i64>,
    pub(crate) litellm_call_id: Option<String>,
    pub(crate) id: Option<String>,
}

// The call's own values, owned so one ContentCall can borrow them for the whole call.
#[derive(Debug)]
pub(crate) struct FileCall {
    pub(crate) file_id: String,
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<BTreeMap<String, String>>,
    pub(crate) chunk_size: u64,
    pub(crate) stream: bool,
    pub(crate) kwargs: ContentKwargs,
}

impl FileCall {
    pub(crate) fn call(&self) -> ContentCall<'_> {
        ContentCall {
            file_id: &self.file_id,
            model: self.model.as_deref(),
            custom_llm_provider: self.custom_llm_provider.clone(),
            extra_headers: self.extra_headers.clone(),
            extra_body: self.extra_body.clone(),
            chunk_size: self.chunk_size,
            stream: self.stream,
            kwargs: &self.kwargs,
        }
    }
}

// The buffered answer: the response status, its headers and the body the caller gets as a blob.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PlainAnswer {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
}

// The streamed answer: the provider's response headers, with the chunks behind the native stream.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct StreamAnswer {
    pub(crate) headers: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the file content payload is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file content answer could not be written: {source}")]
    Write {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file content call's timeout is not usable: expected {0}")]
    TimeoutShapes(&'static str),
    #[error("the file content call's timeout phases are not usable: {source}")]
    Phases {
        #[source]
        source: serde_json::Error,
    },
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

pub(crate) fn parse_call(text: &str) -> Result<FileCall, ConvertError> {
    let wire: RequestWire =
        serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
    build_call(wire)
}

fn build_call(wire: RequestWire) -> Result<FileCall, ConvertError> {
    let timeout = wire.timeout.as_ref().map(read_timeout).transpose()?;
    let request_timeout = wire
        .request_timeout
        .as_ref()
        .map(|value| {
            plain_timeout(value).ok_or(ConvertError::TimeoutShapes(REQUEST_TIMEOUT_SHAPES))
        })
        .transpose()?;
    let kwargs = ContentKwargs {
        api_key: wire.api_key,
        api_base: wire.api_base,
        api_version: wire.api_version,
        organization: wire.organization,
        timeout,
        request_timeout,
        max_retries: wire.max_retries,
        litellm_call_id: wire.litellm_call_id,
        id: wire.id,
    };
    Ok(FileCall {
        file_id: wire.file_id,
        model: wire.model,
        custom_llm_provider: wire.custom_llm_provider,
        extra_headers: wire.extra_headers,
        extra_body: wire.extra_body,
        chunk_size: wire.chunk_size,
        stream: wire.stream,
        kwargs,
    })
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
fn read_timeout(value: &JsonValue) -> Result<ReadTimeout, ConvertError> {
    if let JsonValue::Object(_) = value {
        let text =
            serde_json::to_string(value).map_err(|source| ConvertError::Phases { source })?;
        let phases =
            serde_json::from_str(&text).map_err(|source| ConvertError::Phases { source })?;
        return Ok(ReadTimeout::Structured(phases));
    }
    plain_timeout(value)
        .map(ReadTimeout::Plain)
        .ok_or(ConvertError::TimeoutShapes(READ_TIMEOUT_SHAPES))
}

fn plain_timeout(value: &JsonValue) -> Option<ReadTimeoutValue> {
    match value {
        JsonValue::Number(number) => Some(ReadTimeoutValue::Number(number.clone())),
        JsonValue::Text(text) => Some(ReadTimeoutValue::Text(text.clone())),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}
