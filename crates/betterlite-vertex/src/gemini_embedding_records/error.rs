use betterlite_models::billing_text::error::BillingTextError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiEmbeddingError {
    #[error("a combined embedding group must hold at least one text")]
    EmptyGroup,
    #[error("the embedContent path does not accept combined embedding groups")]
    NestedGroupUnsupported,
    #[error("the file reference {reference} has no resolved MIME type and URI")]
    UnresolvedFile { reference: String },
    #[error("the data URL does not start with data:")]
    DataUrlPrefix,
    #[error("the data URL has no comma before its payload")]
    DataUrlComma,
    #[error("{mime_type} is not a supported embedding media type")]
    UnsupportedMime { mime_type: String },
    #[error("no supported embedding media type matches the extension of {url}")]
    UnsupportedExtension { url: String },
    #[error("{value} is not a Gemini embedding task type")]
    TaskType { value: String },
    #[error("the embeddings reply must be an object")]
    ReplyNotObject,
    #[error("the {field} value is required in the embeddings reply")]
    MissingField { field: &'static str },
    #[error("the {field} record must be an object")]
    NotObject { field: &'static str },
    #[error("the {field} value must be a list")]
    NotList { field: &'static str },
    #[error("the {field} value must be text")]
    NotText { field: &'static str },
    #[error("the {field} value must be a whole number")]
    NotInteger { field: &'static str },
    #[error("the {field} value must be a list of numbers")]
    NotNumbers { field: &'static str },
    #[error("the token count {value} is outside the supported range")]
    TokenRange { value: i64 },
    #[error("the token counts overflow")]
    TokenOverflow,
    #[error("the embeddings reply holds more entries than the index can address")]
    IndexOverflow,
    #[error("the embeddings text could not be formatted: {source}")]
    Formatting {
        #[source]
        source: BillingTextError,
    },
    #[error("the embeddings text could not be counted: {source}")]
    Counting {
        #[source]
        source: CountError,
    },
}
