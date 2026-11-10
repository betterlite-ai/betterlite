use betterlite_context::error::ContextError;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_prompts::azure::error::AzureError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error(
        "Azure does not support 'tool_choice', for api_version={api_version}. Bump your API version to '2023-12-01-preview' or later. This parameter requires 'api_version=\"2023-12-01-preview\"' or later. Azure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions"
    )]
    ToolChoiceUnsupported { api_version: String },
    #[error(
        "Azure does not support 'required' as a tool_choice param, for api_version={api_version}. To drop 'tool_choice=required' for calls with this Azure API version, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\nAzure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions"
    )]
    RequiredToolChoice { api_version: String },
    #[error("the api_version {field} is not an integer: {value}")]
    ApiVersionNumber { field: &'static str, value: String },
    #[error("the existing tools value is not a list")]
    ToolsNotList,
    #[error("response_format cannot be translated into a tool")]
    ResponseFormat {
        #[source]
        source: ContextError,
    },
    #[error("the messages cannot be converted for Azure")]
    Messages {
        #[source]
        source: AzureError,
    },
    #[error("a converted value cannot be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl PolicyError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ToolChoiceUnsupported { .. } | Self::RequiredToolChoice { .. } => Some(400),
            Self::ApiVersionNumber { .. }
            | Self::ToolsNotList
            | Self::ResponseFormat { .. }
            | Self::Messages { .. }
            | Self::Encode { .. } => None,
        }
    }

    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        let _status = self.status_code()?;
        Some(ErrorClass::UnsupportedParams(Box::new(ErrorContext {
            message: self.to_string(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        })))
    }
}
