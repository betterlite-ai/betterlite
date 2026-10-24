use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlPathError {
    #[error("{field_name} is required")]
    Required { field_name: String },
    #[error("{field_name} cannot be a dot path segment")]
    DotSegment { field_name: String },
}
