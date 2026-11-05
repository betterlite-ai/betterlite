#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("message at index {index} must be an object")]
    MessageNotObject { index: usize },
    #[error("messages exceeds the maximum of {limit}")]
    TooManyMessages { limit: usize },
    #[error("tool_calls exceeds the maximum of {limit}")]
    TooManyToolCalls { limit: usize },
}

pub mod groups;
pub mod text;
pub mod thinking;
pub mod tools;

mod tests;
