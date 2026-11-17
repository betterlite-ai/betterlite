use betterlite_anthropic::anthropic_capability_catalog::forced_tool::ForcedToolChoiceError;
use betterlite_openai::error::OpenAiError;

pub const BAD_REQUEST_STATUS: u16 = 400;

#[derive(Debug, thiserror::Error)]
pub enum ConverseToolsError {
    #[error(
        "Bedrock doesn't support tool_choice=none. To drop it from the call, set `litellm.drop_params = True."
    )]
    UnsupportedNone,
    #[error(
        "Bedrock doesn't support tool_choice={value}. Supported tool_choice values=['auto', 'required', json object]. To drop it from the call, set `litellm.drop_params = True."
    )]
    UnsupportedChoice { value: String },
    #[error(transparent)]
    ForcedTool(#[from] ForcedToolChoiceError),
    #[error("the response_format value is not an object")]
    FormatNotAnObject,
    #[error("response_format has no type field")]
    MissingFormatType,
    #[error("response_format.json_schema is not an object")]
    JsonSchemaNotAnObject,
    #[error("response_format.json_schema has no schema field")]
    MissingSchemaField,
    #[error("response_format json_schema {field} is not a string")]
    NotText { field: &'static str },
    #[error("the tool is not an object")]
    ToolNotAnObject,
    #[error("the tool type is not a string")]
    ToolType,
    #[error("the tool function is not an object")]
    ToolFunction,
    #[error("the tool function parameters are not an object")]
    ToolParameters,
    #[error("the tool parameters are not an object")]
    ParametersNotAnObject,
    #[error("the tool parameters have no type field")]
    MissingParameterType,
    #[error("the tool_choice function is not an object")]
    ToolChoiceFunction,
    #[error("the tool_choice function name is not a string")]
    ToolChoiceName,
    #[error("the existing tools value is not a list")]
    ToolsNotList,
    #[error("the tools could not be appended: {0}")]
    Append(#[source] OpenAiError),
    #[error("the optional params could not cross to the base helper: {0}")]
    Wire(#[source] serde_json::Error),
}

impl From<OpenAiError> for ConverseToolsError {
    fn from(error: OpenAiError) -> Self {
        if matches!(error, OpenAiError::ToolsNotList) {
            return Self::ToolsNotList;
        }
        Self::Append(error)
    }
}

impl ConverseToolsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedNone
            | Self::UnsupportedChoice { .. }
            | Self::FormatNotAnObject
            | Self::MissingFormatType
            | Self::JsonSchemaNotAnObject
            | Self::MissingSchemaField
            | Self::NotText { .. }
            | Self::ToolNotAnObject
            | Self::ToolType
            | Self::ToolFunction
            | Self::ToolParameters
            | Self::ParametersNotAnObject
            | Self::MissingParameterType
            | Self::ToolChoiceFunction
            | Self::ToolChoiceName
            | Self::ToolsNotList => Some(BAD_REQUEST_STATUS),
            Self::ForcedTool(error) => error.status_code(),
            Self::Append(_) | Self::Wire(_) => None,
        }
    }
}
