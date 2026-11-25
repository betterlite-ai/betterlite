use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

// The parts of the placeholder response LiteLLM attaches to its rejected-provider 400
pub const UNSUPPORTED_BODY: &str = "Unsupported provider";
pub const UNSUPPORTED_URL: &str = "https://github.com/BerriAI/litellm";
pub const UNSUPPORTED_MODEL: &str = "n/a";

// The response LiteLLM builds with httpx.Response and httpx.Request for a rejected provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadRequest<'error> {
    pub model: &'static str,
    pub llm_provider: &'error str,
    pub body: &'static str,
    pub method: &'static str,
    pub url: &'static str,
}

#[derive(Debug, Error)]
pub enum FileContentError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    // LlmProviders(custom_llm_provider), whose ValueError names the repr of an absent or unknown name
    #[error("{provider} is not a valid LlmProviders")]
    UnknownProvider { provider: String },
    #[error(
        "LiteLLM doesn't support {provider} for 'file_content'. Supported providers are 'openai', 'azure', 'vertex_ai', 'bedrock', 'manus', 'anthropic'."
    )]
    Unsupported { provider: String },
    // file_content_streaming's own rejection, which the streaming predicate keeps unreachable
    #[error(
        "LiteLLM doesn't support {provider} for streaming 'file_content'. Supported providers are ['hosted_vllm', 'litellm_proxy', 'openai', 'vertex_ai']."
    )]
    UnsupportedStreaming { provider: String },
    #[error("Streaming 'file_content' for vertex_ai is only supported through 'afile_content'.")]
    SyncVertexStreaming,
    #[error("the file content dispatch answered with another reply kind")]
    Reply,
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> FileContentError<DispatchError>
where
    DispatchError: Error + 'static,
{
    // Only rejected-provider branches carry the synthesized 400; a bad timeout or name is a ValueError
    #[must_use]
    pub const fn bad_request(&self) -> Option<BadRequest<'_>> {
        let (llm_provider, method) = match self {
            Self::Unsupported { provider } | Self::UnsupportedStreaming { provider } => {
                (provider.as_str(), "CREATE_THREAD")
            }
            Self::SyncVertexStreaming => ("vertex_ai", "FILE_CONTENT"),
            Self::Timeout(_) | Self::UnknownProvider { .. } | Self::Reply | Self::Dispatch(_) => {
                return None;
            }
        };
        Some(BadRequest {
            model: UNSUPPORTED_MODEL,
            llm_provider,
            body: UNSUPPORTED_BODY,
            method,
            url: UNSUPPORTED_URL,
        })
    }
}
