use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTokenDetails {
    pub text_tokens: i64,
    pub audio_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: InputTokenDetails,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DurationUsage {
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptionUsage {
    Tokens(Box<TokenUsage>),
    Duration(DurationUsage),
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub const fn duration_usage(seconds: f64) -> TranscriptionUsage {
    TranscriptionUsage::Duration(DurationUsage { seconds })
}
