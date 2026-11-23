use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::passthrough_call::{PassthroughCallArgs, PassthroughUpload};
use betterlite_core::passthrough_stream::error::PassthroughStreamError;
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
const API_ERROR: &str = "APIError";
const BAD_REQUEST: &str = "BadRequestError";
const PLAIN_EXCEPTION: &str = "Exception";
const BAD_REQUEST_STATUS: u16 = 400;
const SYNTHESIZED_STATUS: u16 = 500;

// One llm_passthrough_route call, one field per named parameter of LiteLLM's signature; the
// caller's loose keyword arguments land in extra and reach the call as its kwargs.
#[derive(Debug, Default, Deserialize)]
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
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}

// The wire's file markers become the call's content and multipart uploads, each blob moved out once.
pub(crate) fn call_args(
    text: &str,
    blobs: &mut [Vec<u8>],
) -> Result<PassthroughCallArgs, ConvertError> {
    let request: CallRequest = parse(text)?;
    let content = content_bytes(request.content.as_ref(), blobs)?;
    let files = uploads(request.files.as_ref(), blobs)?;
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
        content,
        data: request.data,
        files,
        json: request.json,
        params: request.params,
        cookies: request.cookies,
        client: None,
        extra: request.extra,
    })
}

fn content_bytes(
    marker: Option<&Json>,
    blobs: &mut [Vec<u8>],
) -> Result<Option<Bytes>, ConvertError> {
    let Some(marker) = marker else {
        return Ok(None);
    };
    let taken = marker_files(CONTENT_KEY, marker, blobs)?;
    let bytes = taken
        .into_iter()
        .next()
        .map(|(_key, file)| Bytes::from(file.bytes));
    bytes.map_or(Err(ConvertError::Content), |body| Ok(Some(body)))
}

fn uploads(
    marker: Option<&Json>,
    blobs: &mut [Vec<u8>],
) -> Result<Option<Vec<PassthroughUpload>>, ConvertError> {
    let Some(marker) = marker else {
        return Ok(None);
    };
    let taken = marker_files(FILES_KEY, marker, blobs)?;
    Ok(Some(taken.into_iter().map(upload).collect()))
}

fn marker_files(
    key: &'static str,
    marker: &Json,
    blobs: &mut [Vec<u8>],
) -> Result<Vec<(String, CallFile)>, ConvertError> {
    let payload = IndexMap::from([(key.to_owned(), marker.clone())]);
    files_from_payload(&payload, blobs).map_err(|source| ConvertError::File { key, source })
}

fn upload((field, file): (String, CallFile)) -> PassthroughUpload {
    PassthroughUpload {
        field,
        filename: file.name,
        content_type: file.content_type,
        bytes: Bytes::from(file.bytes),
    }
}

// The JSON text a NativePassthroughError carries: LiteLLM's exception name and the fields its
// class keeps, so Python rebuilds the same class the rest of the wheel raises.
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
    // _handle_error's provider error: the status, message and body the provider's own class carries
    pub(crate) fn provider(error: &ProviderError, provider: &str, model: &str) -> Self {
        Self {
            kind: API_ERROR.to_owned(),
            message: error.message.clone(),
            status_code: Some(error.status_code),
            llm_provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
            body: error.body.clone().map(JsonValue::Object),
            headers: error.headers.clone().map(pair_map),
        }
    }

    // raise_for_status: the upstream answered past 400 and its body is what the class reports
    pub(crate) fn upstream(status: u16, headers: &[(String, String)], body: &[u8]) -> Self {
        Self {
            kind: API_ERROR.to_owned(),
            message: body_text(body),
            status_code: Some(status),
            llm_provider: None,
            model: None,
            body: serde_json::from_slice(body).ok(),
            headers: Some(pairs(headers)),
        }
    }

    pub(crate) fn transport(message: String) -> Self {
        Self {
            kind: API_ERROR.to_owned(),
            message,
            status_code: Some(SYNTHESIZED_STATUS),
            llm_provider: None,
            model: None,
            body: None,
            headers: None,
        }
    }

    // get_llm_provider refused the model: BadRequestError, as every LiteLLM call reports it
    pub(crate) fn resolution(message: String) -> Self {
        Self {
            kind: BAD_REQUEST.to_owned(),
            message,
            status_code: Some(BAD_REQUEST_STATUS),
            llm_provider: None,
            model: None,
            body: None,
            headers: None,
        }
    }

    // "Provider {custom_llm_provider} not found" is a plain Exception in LiteLLM's route
    pub(crate) fn provider_not_found(provider: &str) -> Self {
        Self {
            kind: PLAIN_EXCEPTION.to_owned(),
            message: format!("Provider {provider} not found"),
            status_code: None,
            llm_provider: None,
            model: None,
            body: None,
            headers: None,
        }
    }

    // the router refused the route before any provider saw it, so its status is the class's
    pub(crate) fn routing(status: u16, message: &str) -> Self {
        Self {
            kind: API_ERROR.to_owned(),
            message: message.to_owned(),
            status_code: Some(status),
            llm_provider: None,
            model: None,
            body: None,
            headers: None,
        }
    }

    pub(crate) fn stream(error: &PassthroughStreamError) -> Self {
        Self::transport(error.to_string())
    }

    // a request text or a keyword argument the native call cannot read
    pub(crate) fn invalid(message: String) -> Self {
        Self::transport(message)
    }
}

fn pairs(headers: &[(String, String)]) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn pair_map(headers: Vec<(String, String)>) -> IndexMap<String, String> {
    headers.into_iter().collect()
}

// the class keeps the upstream body as text; bytes that are not UTF-8 have no text form
fn body_text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(_invalid) => format!(
            "the upstream answered with {} bytes of non-UTF-8 body",
            body.len()
        ),
    }
}
