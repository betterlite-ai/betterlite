use thiserror::Error;

// A kwargs value the model group read cannot take: docs/compatibility.md names the field instead of coercing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelRecognitionError {
    #[error("{field} must be an object")]
    ExpectedObject { field: &'static str },
    #[error("{field} must be a string")]
    InvalidText { field: &'static str },
}
