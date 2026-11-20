use betterlite_openai::cloudflare_chat::error::CloudflareChatError;
use thiserror::Error;

use super::handler::HandlerError;

#[derive(Debug, Error)]
pub enum DispatchError {
    // B5834 replaces these four Perplexity answers with the configs' real wiring
    #[error("{provider} {operation} support is not built yet")]
    NotBuilt {
        provider: &'static str,
        operation: &'static str,
    },
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error(transparent)]
    Cloudflare(#[from] CloudflareChatError),
    #[error("the call could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
