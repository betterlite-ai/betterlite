use crate::converse_params::error::ConverseParamsError;

#[derive(Debug, thiserror::Error)]
pub enum ConverseMessageError {
    #[error("messages[{message}].content has a text part without a text value")]
    TextPartWithoutText { message: usize },
    #[error(
        "Bedrock doesn't support tool calling without `tools=` param specified. Pass `tools=` param OR set `litellm.modify_params = True` // `litellm_settings::modify_params: True` to add dummy tool to the request."
    )]
    ToolCallsWithoutTools,
    #[error(transparent)]
    Params(#[from] ConverseParamsError),
}

impl ConverseMessageError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::TextPartWithoutText { .. } | Self::ToolCallsWithoutTools => Some(400),
            Self::Params(error) => error.status_code(),
        }
    }
}
