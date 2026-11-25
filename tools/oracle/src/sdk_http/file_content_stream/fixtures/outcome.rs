use std::collections::BTreeMap;

use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_core::api::assistant_threads::error::sdk_class;
use betterlite_core::api::file_content_stream::StreamedContent;
use betterlite_core::api::file_content_stream::error::{BadRequest, FileContentError};
use betterlite_core::api::file_http_content::error::ContentError;
use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_openai::files::error::{FilesError, StatusError};
use betterlite_openai::files::records::BinaryContent;
use serde::{Deserialize, Serialize};

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;

use super::fixtures::logging::RecordedIdentity;
use super::wire::ContentWireError;

// the placeholder response LiteLLM attaches to its rejected-provider 400
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub body: String,
    pub method: String,
    pub url: String,
    pub model: String,
    pub provider: String,
}

// HttpxBinaryResponseContent's status, headers and bytes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Buffered {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

// The bytes the wrapped stream yielded, in the order the caller read them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Streamed {
    pub chunks: Vec<Vec<u8>>,
    pub headers: BTreeMap<String, String>,
}

// What file_content_streaming wrote onto the supplied logging object before the provider branch
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub model: String,
    pub details_model: String,
    pub custom_llm_provider: String,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered: Option<Buffered>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed: Option<Streamed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
}

impl StepOutcome {
    pub(super) fn buffered(content: BinaryContent, identity: Option<Identity>) -> Self {
        Self {
            buffered: Some(Buffered {
                status: content.status,
                headers: content.headers.into_iter().collect(),
                body: content.body,
            }),
            streamed: None,
            identity,
            failure: None,
            placeholder: None,
        }
    }

    pub(super) async fn streamed(
        content: StreamedContent,
        identity: Option<Identity>,
    ) -> Result<Self, SdkHttpError> {
        let StreamedContent {
            mut response,
            headers,
        } = content;
        let mut chunks = Vec::new();
        let mut iterator = response
            .async_iter()
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        while let Some(item) = iterator.next_chunk().await {
            match item {
                Ok(bytes) => chunks.push(bytes),
                Err(failure) => {
                    return Err(SdkHttpError::Invalid(format!(
                        "the file stream failed: {failure:?}"
                    )));
                }
            }
        }
        Ok(Self {
            buffered: None,
            streamed: Some(Streamed {
                chunks,
                headers: headers.into_iter().collect(),
            }),
            identity,
            failure: None,
            placeholder: None,
        })
    }

    // LiteLLM's own 400, its ValueErrors and a provider status error all end a step; only the 400 carries a placeholder
    pub(super) fn failed(
        error: &FileContentError<ContentWireError>,
        identity: Option<Identity>,
    ) -> Result<Self, SdkHttpError> {
        let (failure, placeholder) = match error {
            FileContentError::Unsupported { .. }
            | FileContentError::UnsupportedStreaming { .. }
            | FileContentError::SyncVertexStreaming => {
                let (failure, placeholder) = rejected(error)?;
                (failure, Some(placeholder))
            }
            FileContentError::Timeout(_) | FileContentError::UnknownProvider { .. } => {
                (value_failure(error), None)
            }
            FileContentError::Dispatch(wire) => (dispatch_failure(wire)?, None),
            FileContentError::Reply => {
                return Err(SdkHttpError::Invalid(error.to_string()));
            }
        };
        Ok(Self {
            buffered: None,
            streamed: None,
            identity,
            failure: Some(failure),
            placeholder,
        })
    }
}

// the rejected-provider 400: BadRequestError, the message prefix and the placeholder response
fn rejected(
    error: &FileContentError<ContentWireError>,
) -> Result<(Failure, Placeholder), SdkHttpError> {
    let request = error
        .bad_request()
        .ok_or_else(|| SdkHttpError::Invalid(error.to_string()))?;
    let failure = Failure {
        class: String::from("BadRequestError"),
        status: 400,
        kind: Some(None),
        param: Some(None),
        code: Some(None),
        message: format!("litellm.BadRequestError: {error}"),
    };
    Ok((failure, placeholder(request)))
}

fn placeholder(request: BadRequest<'_>) -> Placeholder {
    Placeholder {
        body: request.body.to_owned(),
        method: request.method.to_owned(),
        url: request.url.to_owned(),
        model: request.model.to_owned(),
        provider: request.llm_provider.to_owned(),
    }
}

// LiteLLM's ValueError: no SDK class, a zero status and no type, param or code
fn value_failure(error: &FileContentError<ContentWireError>) -> Failure {
    Failure {
        class: String::from("ValueError"),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}

// a provider status error keeps the SDK's own class, status, type, param, code and message
fn dispatch_failure(wire: &ContentWireError) -> Result<Failure, SdkHttpError> {
    let status: &StatusError = match wire {
        ContentWireError::OpenAi(FilesError::Status(status))
        | ContentWireError::Azure(AzureFilesError::Files {
            source: FilesError::Status(status),
        }) => status.as_ref(),
        ContentWireError::Content(ContentError::Provider(source)) => {
            return Ok(raised(source));
        }
        ContentWireError::OpenAi(_)
        | ContentWireError::Azure(_)
        | ContentWireError::Content(_)
        | ContentWireError::Fixture(_)
        | ContentWireError::Unbuilt { .. }
        | ContentWireError::Reply => {
            return Err(SdkHttpError::Invalid(wire.to_string()));
        }
    };
    Ok(Failure {
        class: sdk_class(status.status).to_owned(),
        status: status.status,
        kind: Some(status.error_type.clone()),
        param: Some(status.param.clone()),
        code: Some(status.code.clone()),
        message: status.message.clone(),
    })
}

// the generic handler's provider error: a statusless ValueError or the provider config's own status error
fn raised(source: &BaseLlmException) -> Failure {
    if source.status_code >= 400 {
        return Failure {
            class: sdk_class(source.status_code).to_owned(),
            status: source.status_code,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: source.message.clone(),
        };
    }
    Failure {
        class: String::from("ValueError"),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: source.message.clone(),
    }
}

// the first identity the supplied object recorded, if file_content_streaming reached the write
pub(super) fn identity(recorded: &[RecordedIdentity]) -> Option<Identity> {
    let identity = recorded.first()?;
    Some(Identity {
        model: identity.model.clone(),
        details_model: identity.model.clone(),
        custom_llm_provider: identity.custom_llm_provider.clone(),
        api_base: identity.api_base.clone(),
    })
}
