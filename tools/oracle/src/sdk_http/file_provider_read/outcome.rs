use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_core::api::assistant_threads::error::sdk_class;
use betterlite_core::api::file_provider_read::error::{
    FileReadError, UNSUPPORTED_BODY, UNSUPPORTED_MODEL, UNSUPPORTED_URL,
};
use betterlite_openai::files::error::{FilesError, StatusError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;

use super::wire::ReadWireError;

// the placeholder response LiteLLM attaches to its unsupported-provider 400
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub body: String,
    pub method: String,
    pub url: String,
    pub model: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
}

impl StepOutcome {
    // LiteLLM's own 400, its ValueErrors and a provider status error all end a step; only the 400 carries a placeholder
    pub(super) fn failed(error: &FileReadError<ReadWireError>) -> Result<Self, SdkHttpError> {
        let (failure, placeholder) = match error {
            FileReadError::Unsupported { provider, function } => (
                Failure {
                    class: "BadRequestError".to_owned(),
                    status: 400,
                    kind: Some(None),
                    param: Some(None),
                    code: Some(None),
                    message: format!("litellm.BadRequestError: {error}"),
                },
                Some(Placeholder {
                    body: UNSUPPORTED_BODY.to_owned(),
                    method: function.request_method().to_owned(),
                    url: UNSUPPORTED_URL.to_owned(),
                    model: UNSUPPORTED_MODEL.to_owned(),
                    provider: provider.clone(),
                }),
            ),
            FileReadError::Timeout(_) | FileReadError::UnknownProvider { .. } => (
                Failure {
                    class: "ValueError".to_owned(),
                    status: 0,
                    kind: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                },
                None,
            ),
            FileReadError::Dispatch(wire) => (dispatch_failure(wire)?, None),
        };
        Ok(Self {
            file: None,
            listing: None,
            failure: Some(failure),
            placeholder,
        })
    }
}

// a provider status error keeps the SDK's own class, status, type, param, code and message
fn dispatch_failure(wire: &ReadWireError) -> Result<Failure, SdkHttpError> {
    let status: &StatusError = match wire {
        ReadWireError::OpenAi(FilesError::Status(status))
        | ReadWireError::Azure(AzureFilesError::Files {
            source: FilesError::Status(status),
        }) => status.as_ref(),
        ReadWireError::OpenAi(_)
        | ReadWireError::Azure(_)
        | ReadWireError::Anthropic(_)
        | ReadWireError::Unbuilt { .. }
        | ReadWireError::Reply
        | ReadWireError::Header { .. } => {
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
