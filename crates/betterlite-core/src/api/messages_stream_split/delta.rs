use serde::{Deserialize, Serialize};

// LiteLLM's StreamingContentBlockDeltaType and the payload key each delta type writes on the Anthropic frame
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingDeltaType {
    TextDelta,
    InputJsonDelta,
    ThinkingDelta,
    SignatureDelta,
}

impl StreamingDeltaType {
    #[must_use]
    pub const fn payload_field(self) -> &'static str {
        match self {
            Self::TextDelta => "text",
            Self::InputJsonDelta => "partial_json",
            Self::ThinkingDelta => "thinking",
            Self::SignatureDelta => "signature",
        }
    }
}
