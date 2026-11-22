use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::api::messages_chat_content::error::ChatContentError;
use crate::api::messages_chat_tools_schema::tools::ToolsError;

#[derive(Debug, Error)]
pub enum SummaryHistoryError {
    #[error(transparent)]
    Messages(#[from] ChatContentError),
    #[error(transparent)]
    Tools(#[from] ToolsError),
    #[error("the summary tag pattern does not compile")]
    Pattern(#[source] regex::Error),
    #[error("the history could not be token-counted")]
    TokenCount(#[from] CountError),
    #[error("a history record could not be re-encoded as JSON")]
    Encode(#[from] serde_json::Error),
}
