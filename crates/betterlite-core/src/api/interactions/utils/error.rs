use thiserror::Error;

#[derive(Debug, Error)]
pub enum InteractionsUtilsError {
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field} must be an object")]
    NotAnObject { field: &'static str },
    #[error("{field} must be a list")]
    NotList { field: &'static str },
}
