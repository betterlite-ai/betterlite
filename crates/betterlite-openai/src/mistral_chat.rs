pub mod messages;
pub mod reply;
pub mod request;
pub mod stream;
pub mod wire;

mod tests;

use thiserror::Error;

use crate::chat_content::ChatContentError;
use crate::compatible_wire::CompatibleWireError;

pub const PROVIDER: &str = "mistral";

#[derive(Debug, Error)]
pub enum MistralChatError {
    #[error("a chat message could not be converted for the prompt helpers: {source}")]
    MessageShape {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error("the provider reply is not valid JSON: {source}")]
    ReplyJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("a reply content block does not have the shape Mistral documents: {source}")]
    ReplyBlock {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider reply could not be re-encoded: {source}")]
    ReplyEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Reply(#[from] CompatibleWireError),
    #[error("the file field of a file content block is not an object")]
    FileBlockShape,
}
