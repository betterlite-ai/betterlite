use thiserror::Error;

#[derive(Debug, Error)]
pub enum SendParamsError {
    #[error("Invalid message/send params: params must be a JSON object")]
    NotAnObject,
    #[error("Invalid message/send params: {field} must be a JSON object")]
    NotAnObjectAt { field: &'static str },
    #[error("Invalid message/send params: {path}: {source}")]
    Malformed {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Invalid message/send params: {field} holds a value a {expected} cannot represent")]
    InvalidValue {
        field: &'static str,
        expected: &'static str,
    },
    #[error("Invalid message/send params: a message part carries no content")]
    MissingPartContent,
    #[error("Invalid message/send params: a message part carries more than one content key")]
    MultiplePartContents,
}
