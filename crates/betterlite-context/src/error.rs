use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("response_format json_schema has no schema")]
    MissingSchema,
    #[error("response_format json_schema is not an object")]
    SchemaNotObject,
    #[error("message {index} has no role")]
    MissingRole { index: usize },
    #[error("developer message {index} has no content")]
    MissingContent { index: usize },
}
