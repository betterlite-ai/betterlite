use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponseParamsError {
    #[error(
        "{model} doesn't support `reasoning.effort` (its model cost map entry lacks `supports_reasoning`). To drop unsupported params set `litellm.drop_params = True`"
    )]
    UnsupportedReasoning { model: String },
    #[error(
        "{model} doesn't support temperature={temperature} while reasoning is active. Only temperature=1 is supported unless reasoning.effort resolves to 'none'. To drop unsupported params set `litellm.drop_params = True`"
    )]
    UnsupportedTemperature { model: String, temperature: Value },
    #[error(
        "{model} only supports top_p when reasoning.effort resolves to 'none'. To drop unsupported params set `litellm.drop_params = True`"
    )]
    UnsupportedTopP { model: String },
}

impl ResponseParamsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedReasoning { .. }
            | Self::UnsupportedTemperature { .. }
            | Self::UnsupportedTopP { .. } => 400,
        }
    }
}
