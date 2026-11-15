use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioChatError {
    #[error("`{field}` is not a finite number")]
    NonFinite { field: String },
}
