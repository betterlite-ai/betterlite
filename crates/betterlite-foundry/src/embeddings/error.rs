use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::embedding::EmbeddingError as TextEmbeddingError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FoundryEmbeddingError {
    #[error(
        "api_base is None. Please set AZURE_AI_API_BASE or dynamically via `api_base` param, to make the request."
    )]
    ApiBaseMissing,
    #[error(
        "api_key is None. Please set AZURE_AI_API_KEY or dynamically via `api_key` param, to make the request."
    )]
    ApiKeyMissing,
    #[error("/image/embeddings route returned None Embeddings.")]
    ImageDataMissing,
    #[error("/v1/embeddings route returned None Embeddings.")]
    TextDataMissing,
    #[error("the provider answer is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer has no data field")]
    DataMissing,
    #[error("the provider reply carries an error with status {status}")]
    ReplyError { status: u16 },
    #[error("the provider answer field {field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the provider answer has too few {entries} entries")]
    TooFewEntries { entries: &'static str },
    #[error("the llm_provider-num_tokens header {value:?} is not a token count")]
    TokenCount { value: String },
    #[error("the embeddings request cannot be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {name} request header cannot be built: {source}")]
    Header {
        name: &'static str,
        #[source]
        source: http::Error,
    },
    #[error("the request to {url} failed")]
    Transport {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("the provider answered with status {status}")]
    Provider { status: u16 },
    #[error(transparent)]
    Text(#[from] TextEmbeddingError),
}
