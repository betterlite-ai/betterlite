use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("a message must be an object")]
    MessageNotObject,
    #[error("a message at index {index} must be an object")]
    MessageAtNotObject { index: usize },
    #[error("a message has no role")]
    MessageWithoutRole,
    #[error("a Bedrock message has no content")]
    MessageWithoutContent,
    #[error("a message content must be a list of blocks")]
    MessageContentNotList,
    #[error("a message at index {index} has content that is not a list of blocks")]
    MessageContentNotListAt { index: usize },
    #[error("a content part at index {index} is not an object")]
    PartNotObject { index: usize },
    #[error("a content part at index {index} has no type")]
    PartWithoutType { index: usize },
    #[error("a text part at index {index} has no text")]
    TextPartWithoutText { index: usize },
    #[error("a text part at index {index} has text that is not a string")]
    TextPartNotText { index: usize },
    #[error("a message content is neither a string nor a list of parts")]
    UnsupportedContent,
    #[error("a Bedrock content block at index {index} is not an object")]
    BlockNotObject { index: usize },
    #[error("a document block has a name that is not a string")]
    DocumentNameNotText,
    #[error("a {block_key} block has an id that is not a string")]
    BlockIdNotText { block_key: String },
    #[error("a continue message must be an object")]
    ContinueMessageNotObject,
    #[error("a duplicate document name counter overflowed")]
    CounterOverflow,
}
