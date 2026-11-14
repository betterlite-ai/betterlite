use thiserror::Error;

use crate::chat_content::ChatContentError;

#[derive(Debug, Error)]
pub enum HostedVllmChatError {
    #[error("a request parameter could not be written as JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("tool {index} is not a JSON object")]
    ToolNotObject { index: usize },
    #[error("the custom field of a custom tool must be an object")]
    CustomToolObject,
    #[error("the {field} field of a custom tool must be a string")]
    CustomToolString { field: &'static str },
    #[error("the input_schema field of a custom tool must be an object")]
    CustomToolSchema,
    #[error("the {field} field of a {part} content block must be a string")]
    PartString {
        part: &'static str,
        field: &'static str,
    },
    #[error("the file field of a file content block must be an object")]
    FileObject,
    #[error("file_id or file_data is required")]
    MissingVideoSource,
    #[error("the thinking.budget_tokens field must be an integer")]
    ThinkingBudget,
    #[error(transparent)]
    Content(#[from] ChatContentError),
}
