use quick_xml::encoding::EncodingError;
use quick_xml::escape::EscapeError;

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the AWS {operation} request body could not be serialized")]
    Serialize {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the AWS {operation} answer is not UTF-8")]
    NotUtf8 { operation: &'static str },
    #[error("the AWS {operation} answer is not well-formed XML")]
    Xml {
        operation: &'static str,
        #[source]
        source: quick_xml::Error,
    },
    #[error("the AWS {operation} answer has text that is not UTF-8")]
    Decode {
        operation: &'static str,
        #[source]
        source: EncodingError,
    },
    #[error("the AWS {operation} answer has an invalid XML entity")]
    Escape {
        operation: &'static str,
        #[source]
        source: EscapeError,
    },
    #[error("the AWS {operation} answer has no {field} element")]
    MissingField {
        operation: &'static str,
        field: &'static str,
    },
    #[error("the AWS {operation} answer is not valid JSON")]
    Json {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the AWS {operation} answer has no {field}")]
    MissingEntry {
        operation: &'static str,
        field: &'static str,
    },
    #[error("the collection endpoint {endpoint} is not an https host in this partition")]
    CollectionEndpoint { endpoint: String },
}
