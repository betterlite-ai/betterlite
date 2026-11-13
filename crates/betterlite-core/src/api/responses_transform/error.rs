use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponsesTransformError {
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("{field} must be a boolean")]
    NotBool { field: &'static str },
    #[error("{field} must be a number")]
    NotNumber { field: &'static str },
    #[error("{field} must be an integer")]
    NotInteger { field: &'static str },
    #[error("{field} must be an object")]
    NotObject { field: &'static str },
    #[error("{field} must be a list")]
    NotList { field: &'static str },
    #[error("{field} must be a string, an object or a list of them")]
    NotInput { field: &'static str },
    #[error("the input content item must be text or an object")]
    NotContentItem,
    #[error("the response creation time {seconds} is out of range")]
    CreatedAt {
        seconds: i64,
        #[source]
        source: jiff::Error,
    },
}
