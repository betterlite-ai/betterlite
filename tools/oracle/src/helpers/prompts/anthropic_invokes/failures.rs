use betterlite_prompts::anthropic_invokes::documents::FileMessageError;
use betterlite_prompts::anthropic_results::images::ImageError;

use crate::helpers::HelperError;

pub(super) fn helper_failure(status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicInvokes".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// LiteLLM's missing-file and download failures are BadRequestErrors; its other file errors escape without a status
pub(super) fn file_failure(error: &FileMessageError) -> Box<HelperError> {
    let status = match error {
        FileMessageError::MissingFile | FileMessageError::Image(ImageError::Fetch(_)) => Some(400),
        FileMessageError::Image(
            ImageError::DataUrl | ImageError::Input | ImageError::Field { .. },
        )
        | FileMessageError::Field { .. }
        | FileMessageError::MissingSource => None,
    };
    helper_failure(status, error.to_string())
}
