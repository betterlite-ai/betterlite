use thiserror::Error;

use crate::bedrock_content::error::ContentError;
use crate::bedrock_images::error::BedrockImageError;
use crate::bedrock_tool_results::error::BedrockToolResultsError;

#[derive(Debug, Error)]
pub enum ProcessorError {
    #[error("a bedrock conversation needs at least one non-system message")]
    NoMessages,
    #[error("a message at index {message} is not an object")]
    MessageNotObject { message: usize },
    #[error("a message at index {message} has no role")]
    MessageWithoutRole { message: usize },
    #[error("the role of the message at index {message} is not a string")]
    RoleNotText { message: usize },
    #[error("the role {role:?} of the message at index {message} cannot start a Bedrock turn")]
    UnsupportedRole { message: usize, role: String },
    #[error("the prefix of the message at index {message} is not a boolean")]
    PrefixNotBool { message: usize },
    #[error("the content of the message at index {message} is neither a string nor a list")]
    ContentNotTextOrList { message: usize },
    #[error("the merged message at index {message} has content that is not a list")]
    MergedContentNotList { message: usize },
    #[error("the thinking_blocks of the message at index {message} is not a list")]
    ThinkingBlocksNotList { message: usize },
    #[error("the tool_calls of the message at index {message} is not a list")]
    ToolCallsNotList { message: usize },
    #[error("the tool message at index {message} does not have a Bedrock shape")]
    ToolMessageShape {
        message: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("a converted Bedrock block could not be serialized")]
    Serialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("a content block at index {block} is not an object")]
    BlockNotObject { block: usize },
    #[error("a content block at index {block} has no type")]
    BlockWithoutType { block: usize },
    #[error("a content block at index {block} has a type that is not a string")]
    BlockTypeNotText { block: usize },
    #[error("a text block at index {block} has no text")]
    TextWithoutText { block: usize },
    #[error("a text block at index {block} has text that is not a string")]
    TextNotText { block: usize },
    #[error("the block at index {block} has no image_url")]
    ImageUrlMissing { block: usize },
    #[error("the image_url of the block at index {block} is not a string or an object")]
    ImageUrlNotTextOrObject { block: usize },
    #[error("the image_url of the block at index {block} has no url")]
    ImageUrlWithoutAddress { block: usize },
    #[error("the image_url of the block at index {block} has a url that is not a string")]
    ImageUrlNotText { block: usize },
    #[error("the image_url of the block at index {block} has a format that is not a string")]
    ImageFormatNotText { block: usize },
    #[error("the file block at index {block} is missing its file field")]
    FileObjectMissing { block: usize },
    #[error("the file field of the block at index {block} is not an object")]
    FileNotObject { block: usize },
    #[error("the file of the block at index {block} has no file_data and no file_id")]
    FileWithoutMedia { block: usize },
    #[error("the file of the block at index {block} has file_data that is not a string")]
    FileDataNotText { block: usize },
    #[error("the file of the block at index {block} has a file_id that is not a string")]
    FileIdNotText { block: usize },
    #[error("the file of the block at index {block} has a format that is not a string")]
    FileFormatNotText { block: usize },
    #[error("the document block at index {block} has no source")]
    DocumentSourceMissing { block: usize },
    #[error("the document source at index {block} is not an object")]
    DocumentSourceNotObject { block: usize },
    #[error("the document source at index {block} is not base64")]
    DocumentSourceNotBase64 { block: usize },
    #[error("the document source at index {block} has no media_type")]
    DocumentMediaTypeMissing { block: usize },
    #[error("the document source at index {block} has a media_type that is not a string")]
    DocumentMediaTypeNotText { block: usize },
    #[error("the document source at index {block} has a media_type without a subtype")]
    DocumentMediaWithoutSubtype { block: usize },
    #[error("the document source at index {block} has no data")]
    DocumentDataMissing { block: usize },
    #[error("the document source at index {block} has data that is not a string")]
    DocumentDataNotText { block: usize },
    #[error("the document block at index {block} could not be formatted")]
    DocumentFormat {
        block: usize,
        #[source]
        source: BedrockImageError,
    },
    #[error("the media of the block at index {block} could not be converted")]
    Image {
        block: usize,
        #[source]
        source: BedrockImageError,
    },
    #[error("a thinking block at index {block} is not an object")]
    ThinkingNotObject { block: usize },
    #[error("a thinking block at index {block} has thinking that is not a string")]
    ThinkingNotText { block: usize },
    #[error("a thinking block at index {block} has a signature that is not a string")]
    ThinkingSignatureNotText { block: usize },
    #[error("a reasoning block at index {block} is not an object")]
    ReasoningNotObject { block: usize },
    #[error("the reasoning content at index {block} is not an object")]
    ReasoningContentNotObject { block: usize },
    #[error("the reasoning text at index {block} is not an object")]
    ReasoningTextNotObject { block: usize },
    #[error("the reasoning text at index {block} has no text")]
    ReasoningTextWithoutText { block: usize },
    #[error("the reasoning text at index {block} has text that is not a string")]
    ReasoningTextNotText { block: usize },
    #[error("the reasoning text at index {block} has a signature that is not a string")]
    ReasoningSignatureNotText { block: usize },
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    ToolInvoke {
        #[from]
        source: BedrockToolResultsError,
    },
    #[error(transparent)]
    ToolResult { source: BedrockToolResultsError },
}
