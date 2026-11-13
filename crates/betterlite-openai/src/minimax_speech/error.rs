use http::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MinimaxSpeechError {
    #[error("MiniMax API key is required. Set MINIMAX_API_KEY or pass api_key")]
    MissingApiKey,
    #[error("the authorization header cannot carry the MiniMax API key")]
    Authorization {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {name} option holds a value MiniMax cannot use")]
    Option { name: &'static str },
    #[error("the MiniMax speech reply is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the MiniMax speech reply is not a JSON object")]
    NotObject,
    #[error("the MiniMax speech reply has a {field} field of the wrong type")]
    Field { field: &'static str },
    #[error("the MiniMax speech reply audio is neither hex nor base64")]
    Audio {
        #[source]
        source: base64::DecodeError,
    },
    #[error("{message}")]
    Reply { status: u16, message: String },
}

impl MinimaxSpeechError {
    // LiteLLM turns a caller value it cannot use into a 400 and every reply it cannot read into a 500
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey | Self::Authorization { .. } => None,
            Self::Option { .. } => Some(400),
            Self::Decode { .. } | Self::NotObject | Self::Field { .. } | Self::Audio { .. } => {
                Some(500)
            }
            Self::Reply { status, .. } => Some(*status),
        }
    }
}
