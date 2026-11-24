use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) endpoint_type: EndpointName,
    pub(crate) chunks: Vec<Vec<u8>>,
    pub(crate) failure: Option<FailureSpec>,
    pub(crate) disconnect_after: Option<usize>,
    pub(crate) status_code: u16,
    pub(crate) cost_injection: Flag,
    pub(crate) request_body: Option<Value>,
    pub(crate) url_route: String,
    pub(crate) deferred: bool,
    pub(crate) tinyfish_poller: bool,
    pub(crate) run_id: Option<String>,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct Flag(pub(crate) bool);

#[derive(Debug, Deserialize)]
pub(crate) struct FailureSpec {
    pub(crate) message: String,
    pub(crate) traceback: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndpointName {
    #[serde(rename = "vertex-ai")]
    VertexAi,
    Gemini,
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

impl From<EndpointName> for EndpointType {
    fn from(name: EndpointName) -> Self {
        match name {
            EndpointName::VertexAi => Self::VertexAi,
            EndpointName::Gemini => Self::Gemini,
            EndpointName::Anthropic => Self::Anthropic,
            EndpointName::OpenAi => Self::OpenAi,
            EndpointName::Tinyfish => Self::Tinyfish,
            EndpointName::Generic => Self::Generic,
            EndpointName::Decisions => Self::Decisions,
        }
    }
}
