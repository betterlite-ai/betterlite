use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeepInfraError {
    #[error(
        "DeepInfra doesn't support this tool_choice value; set drop_params to drop unsupported parameters"
    )]
    UnsupportedToolChoice,
    #[error("DeepInfra does not capture a chat setting named {name}")]
    UnknownSetting { name: String },
    #[error("the minimum non-zero temperature cannot be written as a JSON number")]
    UnrepresentableMinimumTemperature,
}

impl DeepInfraError {
    // LiteLLM raises UnsupportedParamsError with status 400 and lets a TypeError from the constructor surface as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedToolChoice => 400,
            Self::UnknownSetting { .. } | Self::UnrepresentableMinimumTemperature => 500,
        }
    }
}
