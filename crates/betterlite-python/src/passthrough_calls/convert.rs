use std::str::from_utf8;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::passthrough_call::{PassthroughCallArgs, PassthroughUpload};
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Object;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use bytes::Bytes;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod tests;

const CONTENT_KEY: &str = "content";
const FILES_KEY: &str = "files";
pub(crate) const API_ERROR: &str = "APIError";
pub(crate) const BAD_REQUEST: &str = "BadRequestError";
const PLAIN_EXCEPTION: &str = "Exception";
pub(crate) const BAD_REQUEST_STATUS: u16 = 400;
pub(crate) const SYNTHESIZED_STATUS: u16 = 500;

// One llm_passthrough_route call, one field per named parameter of LiteLLM's signature.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallRequest {
    pub(crate) method: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) request_query_params: Option<IndexMap<String, String>>,
    pub(crate) request_headers: Option<IndexMap<String, String>>,
    pub(crate) content: Option<Json>,
    pub(crate) data: Option<Object>,
    pub(crate) files: Option<Json>,
    pub(crate) json: Option<JsonValue>,
    pub(crate) params: Option<Vec<(String, String)>>,
    pub(crate) cookies: Option<Vec<(String, String)>>,
    // the caller's loose keyword arguments reach the call as its kwargs
    #[serde(flatten)]
    pub(crate) extra: Object,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {key} argument is not a file: {source}")]
    File {
        key: &'static str,
        #[source]
        source: CallFilesError,
    },
    #[error("the content argument is not a file marker")]
    Content,
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}

// the other direction of the same boundary: the helper's answer back to the Python caller as JSON text
pub(crate) fn encode<T>(value: &T) -> Result<String, ConvertError>
where
    T: Serialize,
{
    match serde_json::to_string(value) {
        Ok(text) => Ok(text),
        Err(source) => Err(ConvertError::Encode { source }),
    }
}

// The wire's file markers become the call's content and multipart uploads, each blob moved out once.
pub(crate) fn call_args(
    text: &str,
    blobs: &mut [Vec<u8>],
) -> Result<PassthroughCallArgs, ConvertError> {
    let request: CallRequest = parse(text)?;
    let body = call_body(&request, blobs)?;
    Ok(PassthroughCallArgs {
        method: request.method,
        endpoint: request.endpoint,
        model: request.model,
        custom_llm_provider: request.custom_llm_provider,
        api_base: request.api_base,
        api_key: request.api_key.map(SecretString::from),
        request_query_params: request.request_query_params,
        request_headers: request
            .request_headers
            .map(|headers| headers.into_iter().collect()),
        content: body.content,
        data: request.data,
        files: body.files,
        json: request.json,
        params: request.params,
        cookies: request.cookies,
        client: None,
        extra: request.extra,
    })
}

struct CallBody {
    content: Option<Bytes>,
    files: Option<Vec<PassthroughUpload>>,
}

fn marker_files(
    key: &'static str,
    marker: &Json,
    blobs: &mut [Vec<u8>],
) -> Result<Vec<(String, CallFile)>, ConvertError> {
    let payload = IndexMap::from([(key.to_owned(), marker.clone())]);
    match files_from_payload(&payload, blobs) {
        Ok(files) => Ok(files),
        Err(source) => Err(ConvertError::File { key, source }),
    }
}

fn call_body(request: &CallRequest, blobs: &mut [Vec<u8>]) -> Result<CallBody, ConvertError> {
    let content = match request.content.as_ref() {
        None => None,
        Some(marker) => {
            let mut taken = marker_files(CONTENT_KEY, marker, blobs)?;
            let (_key, file) = taken.pop().ok_or(ConvertError::Content)?;
            Some(Bytes::from(file.bytes))
        }
    };
    let files = match request.files.as_ref() {
        None => None,
        Some(marker) => {
            let taken = marker_files(FILES_KEY, marker, blobs)?;
            Some(taken.into_iter().map(upload).collect())
        }
    };
    Ok(CallBody { content, files })
}

fn upload((field, file): (String, CallFile)) -> PassthroughUpload {
    PassthroughUpload {
        field,
        filename: file.name,
        content_type: file.content_type,
        bytes: Bytes::from(file.bytes),
    }
}

// The JSON text a NativePassthroughError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Serialize)]
pub(crate) struct ErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<JsonValue>,
    pub(crate) headers: Option<IndexMap<String, String>>,
}

impl ErrorPayload {
    fn of_kind(kind: &'static str) -> Self {
        Self {
            kind: kind.to_owned(),
            message: String::new(),
            status_code: None,
            llm_provider: None,
            model: None,
            body: None,
            headers: None,
        }
    }

    // _handle_error's provider error: the status, message and body the provider's own class carries
    pub(crate) fn provider(error: &ProviderError, provider: &str, model: &str) -> Self {
        Self {
            message: error.message.clone(),
            status_code: Some(error.status_code),
            llm_provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
            body: error.body.clone().map(JsonValue::Object),
            headers: error
                .headers
                .clone()
                .map(|headers| headers.into_iter().collect()),
            ..Self::of_kind(API_ERROR)
        }
    }

    // raise_for_status: the upstream answered past 400 and its body is what the class reports
    pub(crate) fn upstream(status: u16, headers: &[(String, String)], body: &[u8]) -> Self {
        Self {
            message: body_text(body),
            status_code: Some(status),
            body: serde_json::from_slice(body).ok(),
            headers: Some(pairs(headers)),
            ..Self::of_kind(API_ERROR)
        }
    }

    // a failure with no provider fields of its own: the class and status the route reports
    pub(crate) fn failed(kind: &'static str, status: Option<u16>, message: String) -> Self {
        Self {
            message,
            status_code: status,
            ..Self::of_kind(kind)
        }
    }

    // "Provider {custom_llm_provider} not found" is a plain Exception in LiteLLM's route
    pub(crate) fn provider_not_found(provider: &str) -> Self {
        Self {
            message: format!("Provider {provider} not found"),
            ..Self::of_kind(PLAIN_EXCEPTION)
        }
    }

    // the router refused the route before any provider saw it, so its status is the class's
    pub(crate) fn routing(status: u16, message: &str) -> Self {
        Self {
            message: message.to_owned(),
            status_code: Some(status),
            ..Self::of_kind(API_ERROR)
        }
    }
}

fn pairs(headers: &[(String, String)]) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// the class keeps the upstream body as text; bytes that are not UTF-8 have no text form
fn body_text(body: &[u8]) -> String {
    match from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(_invalid) => format!(
            "the upstream answered with {} bytes of non-UTF-8 body",
            body.len()
        ),
    }
}
