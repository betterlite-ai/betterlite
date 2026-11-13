use std::error::Error as StdError;
use std::fmt::Display;

use betterlite_prompts::anthropic_invokes::documents::FileMessageError;
use betterlite_prompts::anthropic_results::images::ImageError;
use betterlite_prompts::anthropic_results::tool_result::ToolResultError;
use betterlite_prompts::anthropic_user_blocks::error::UserBlocksError;
use betterlite_prompts::anthropic_user_blocks::prelude::PrepareError;

use crate::helpers::HelperError;

pub(crate) fn failure(status: Option<u16>, error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicUserBlocks".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) fn prepare_failure(error: &PrepareError) -> Box<HelperError> {
    failure(
        matches!(error, PrepareError::NoUserMessages).then_some(400),
        error,
    )
}

// LiteLLM's image and missing-file failures are BadRequestErrors; its other conversion errors escape without a status
pub(crate) fn conversion_failure<FileFailure>(
    error: &UserBlocksError<FileFailure>,
) -> Box<HelperError>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    let status = match error {
        UserBlocksError::Image(ImageError::Fetch(_))
        | UserBlocksError::ToolResult(ToolResultError::Image(ImageError::Fetch(_)))
        | UserBlocksError::File(
            FileMessageError::MissingFile | FileMessageError::Image(ImageError::Fetch(_)),
        ) => Some(400),
        UserBlocksError::Image(_)
        | UserBlocksError::File(_)
        | UserBlocksError::ToolResult(_)
        | UserBlocksError::MessageNotObject { .. }
        | UserBlocksError::MissingRole { .. }
        | UserBlocksError::BlockNotObject { .. }
        | UserBlocksError::Field { .. }
        | UserBlocksError::TooManyParts { .. }
        | UserBlocksError::MessageDecode { .. } => None,
    };
    failure(status, error)
}
