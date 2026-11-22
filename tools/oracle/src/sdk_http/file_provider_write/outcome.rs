use betterlite_anthropic::files::error::FilesError as AnthropicError;
use betterlite_core::api::file_provider_write::FileWriteError;
use betterlite_openai::files::error::FilesError;
use serde::Serialize;

use super::super::error::SdkHttpError;
use super::super::result::Failure;
use super::providers::WireError;
use super::{Rejection, StepOutcome};
pub(super) fn step_outcome<Value>(
    result: Result<Value, FileWriteError<WireError>>,
) -> Result<StepOutcome, SdkHttpError>
where
    Value: Serialize,
{
    match result {
        Ok(value) => Ok(StepOutcome {
            value: Some(serde_json::to_value(value).map_err(SdkHttpError::Serialize)?),
            failure: None,
            bad_request: None,
        }),
        Err(error) => Ok(StepOutcome {
            value: None,
            failure: Some(failure(&error)),
            bad_request: error.bad_request().map(|rejected| Rejection {
                model: rejected.model.to_owned(),
                llm_provider: rejected.llm_provider.to_owned(),
                body: rejected.body.to_owned(),
                method: rejected.method.to_owned(),
                url: rejected.url.to_owned(),
            }),
        }),
    }
}
fn failure(error: &FileWriteError<WireError>) -> Failure {
    match error {
        // LiteLLM's BadRequestError names no type, param or code and prefixes its class to the message
        FileWriteError::Passthrough { .. }
        | FileWriteError::UnsupportedUpload { .. }
        | FileWriteError::UnsupportedDelete { .. } => Failure {
            class: "BadRequestError".to_owned(),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("litellm.BadRequestError: {error}"),
        },
        FileWriteError::LoggingObjectRequired
        | FileWriteError::TimeoutText { .. }
        | FileWriteError::Timeout(_)
        | FileWriteError::UnknownProvider { .. } => plain_failure("ValueError", error.to_string()),
        // the openai client's status error is raised as is by the openai branch
        FileWriteError::Provider(WireError::OpenAi(FilesError::Status(status))) => Failure {
            class: class_for_status(status.status).to_owned(),
            status: status.status,
            kind: Some(status.error_type.clone()),
            param: Some(status.param.clone()),
            code: Some(status.code.clone()),
            message: status.message.clone(),
        },
        // the generic handler raises the config's BaseLLMException with the upstream status and text
        FileWriteError::Provider(WireError::Anthropic(AnthropicError::Provider(provider))) => {
            Failure {
                class: "AnthropicError".to_owned(),
                status: provider.status_code,
                kind: None,
                param: None,
                code: None,
                message: provider.message.clone(),
            }
        }
        FileWriteError::Provider(wire) => plain_failure("APIConnectionError", wire.to_string()),
    }
}
fn plain_failure(class: &str, message: String) -> Failure {
    Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message,
    }
}
// the classes the openai client raises for an HTTP status
const fn class_for_status(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIStatusError",
    }
}
