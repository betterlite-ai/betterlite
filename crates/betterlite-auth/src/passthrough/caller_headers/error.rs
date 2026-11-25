use http::StatusCode;

pub const VERTEX_MISSING_CREDENTIAL: &str = "No Vertex AI credential is configured on this proxy and the request carried no upstream Google credential. The LiteLLM virtual key is not forwarded to Google. Configure a Vertex credential (DEFAULT_VERTEXAI_PROJECT / DEFAULT_VERTEXAI_LOCATION / DEFAULT_VERTEXAI_CREDENTIALS, or a model with use_in_pass_through: true), or send your own Google OAuth token in the Authorization header.";

pub const ANTHROPIC_MISSING_CREDENTIAL: &str = "No Anthropic credential is configured on this proxy and the request carried no upstream Anthropic credential. The LiteLLM virtual key is not forwarded to Anthropic. Configure an Anthropic credential (ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN, or a model with use_in_pass_through: true), or send your own Anthropic API key in the x-api-key header or your own Anthropic OAuth token in the Authorization header.";

#[derive(Debug, thiserror::Error)]
pub enum CallerHeaderError {
    #[error("{}", VERTEX_MISSING_CREDENTIAL)]
    VertexCredentialMissing,
    #[error("{}", ANTHROPIC_MISSING_CREDENTIAL)]
    AnthropicCredentialMissing,
}

impl CallerHeaderError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::VertexCredentialMissing | Self::AnthropicCredentialMissing => {
                StatusCode::UNAUTHORIZED
            }
        }
    }
}
