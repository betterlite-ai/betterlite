use betterlite_prompts::anthropic_results::function_invoke::InvokeError;
use betterlite_prompts::anthropic_results::images::ImageError;
use betterlite_prompts::anthropic_results::tool_result::ToolResultError;

use crate::helpers::HelperError;

pub(super) fn helper_failure(status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicResults".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn image_failure(error: &ImageError) -> Box<HelperError> {
    let status = match error {
        ImageError::Fetch(_) => Some(400),
        ImageError::DataUrl | ImageError::Input | ImageError::Field { .. } => None,
    };
    helper_failure(status, error.to_string())
}

pub(super) fn tool_result_failure<Failure>(error: &ToolResultError<Failure>) -> Box<HelperError>
where
    Failure: std::error::Error + Send + Sync + 'static,
{
    let status = match error {
        ToolResultError::Image(ImageError::Fetch(_)) => Some(400),
        ToolResultError::Image(
            ImageError::DataUrl | ImageError::Input | ImageError::Field { .. },
        )
        | ToolResultError::Role { .. }
        | ToolResultError::MissingToolCallId
        | ToolResultError::Field { .. }
        | ToolResultError::TooManyParts { .. }
        | ToolResultError::File(_) => None,
    };
    helper_failure(status, error.to_string())
}

pub(super) fn invoke_failure(error: &InvokeError) -> Box<HelperError> {
    let status = match error {
        InvokeError::Arguments(_) | InvokeError::Field { .. } => None,
    };
    helper_failure(status, error.to_string())
}
