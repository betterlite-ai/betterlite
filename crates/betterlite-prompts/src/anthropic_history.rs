#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("message at index {index} must be an object")]
    MessageNotObject { index: usize },
}

pub mod groups;
pub mod text;
pub mod thinking;
pub mod tools;

mod tests;
