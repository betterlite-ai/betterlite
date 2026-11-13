#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("the guardrail returned item {response_index}, which has no recorded position")]
    UnmappedResponse { response_index: usize },
    #[error("there is no message or choice at index {index}")]
    MissingEntry { index: usize },
    #[error("entry {index} has no content item {content_index} to rewrite")]
    MissingContentItem { index: usize, content_index: usize },
}
