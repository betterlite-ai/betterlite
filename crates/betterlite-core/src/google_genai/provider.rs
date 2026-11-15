use betterlite_logging::passthrough_stream_logging::types::EndpointType;

// LiteLLM logs only the exact "gemini" spelling as Gemini and every other provider spelling as Vertex
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenAiProvider {
    Gemini,
    VertexAi,
}

impl GenAiProvider {
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Self {
        if spelling == "gemini" {
            Self::Gemini
        } else {
            Self::VertexAi
        }
    }

    #[must_use]
    pub const fn endpoint_type(self) -> EndpointType {
        match self {
            Self::Gemini => EndpointType::Gemini,
            Self::VertexAi => EndpointType::VertexAi,
        }
    }
}
