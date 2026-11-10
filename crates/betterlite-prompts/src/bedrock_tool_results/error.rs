use thiserror::Error;

use crate::bedrock_images::error::BedrockImageError;

#[derive(Debug, Error)]
pub enum BedrockToolResultsError {
    #[error("a tool call at index {index} is not an object")]
    ToolCallNotObject { index: usize },
    #[error("the function of the tool call at index {index} is not an object")]
    FunctionNotObject { index: usize },
    #[error("the tool call at index {index} has no id")]
    ToolCallWithoutId { index: usize },
    #[error("the tool call at index {index} has an id that is not a string")]
    ToolCallIdNotText { index: usize },
    #[error("the function of the tool call at index {index} has a name that is not a string")]
    FunctionNameNotText { index: usize },
    #[error("the function of the tool call at index {index} has arguments that are not a string")]
    ArgumentsNotText { index: usize },
    #[error("a tool result content part at index {index} has no text")]
    TextPartWithoutText { index: usize },
    #[error("a tool result content part at index {index} has text that is not a string")]
    TextPartNotText { index: usize },
    #[error("a tool result content part at index {index} has no image_url")]
    ImageUrlWithoutUrl { index: usize },
    #[error(
        "a tool result content part at index {index} has an image_url that is not a string or an object"
    )]
    ImageUrlNotTextOrObject { index: usize },
    #[error("the image_url of the part at index {index} has no url")]
    ImageUrlWithoutAddress { index: usize },
    #[error("the image_url of the part at index {index} has a url that is not a string")]
    ImageUrlNotText { index: usize },
    #[error("the image_url of the part at index {index} has a format that is not a string")]
    ImageFormatNotText { index: usize },
    #[error("the file of the part at index {index} is not an object")]
    FileNotObject { index: usize },
    #[error("the file of the part at index {index} has no file_data and no file_id")]
    FileWithoutMedia { index: usize },
    #[error("the file of the part at index {index} has a file_data that is not a string")]
    FileDataNotText { index: usize },
    #[error("the file of the part at index {index} has a file_id that is not a string")]
    FileIdNotText { index: usize },
    #[error("the file of the part at index {index} has a format that is not a string")]
    FileFormatNotText { index: usize },
    #[error("the tool result message has a tool_call_id that is not a string")]
    ResultToolCallIdNotText,
    #[error("the tool result media could not be converted")]
    Image {
        #[source]
        source: BedrockImageError,
    },
}
