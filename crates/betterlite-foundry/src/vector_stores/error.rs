use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use thiserror::Error;

// B1058's executor protocol returns a plain exception at the Python seam, so the injected call keeps its text here
#[derive(Debug, Error)]
pub enum EmbeddingCallError {
    #[error("{message}")]
    Provider { message: String },
}

#[derive(Debug, Error)]
pub enum AzureSearchError {
    #[error("api_key is required")]
    MissingApiKey,
    #[error(
        "Azure AI Search service name is required. Provide it via litellm_params['azure_search_service_name'] or api_base parameter"
    )]
    MissingServiceName,
    #[error(
        "litellm_embedding_model is required in litellm_params for this vector store. Example: litellm_params['litellm_embedding_model'] = 'openai/text-embedding-3-small'"
    )]
    MissingEmbeddingModel,
    #[error("{field} must be {expected}")]
    InvalidField {
        field: &'static str,
        expected: &'static str,
    },
    #[error("Failed to generate embedding for query: {source}")]
    Embedding {
        #[source]
        source: EmbeddingCallError,
    },
    #[error("the embedding response holds no first record")]
    NoFirstRecord,
    #[error("the first embedding record is not a vector of numbers")]
    NotAVector,
    #[error("the Azure AI Search reply is not valid JSON")]
    ReplyJson {
        status_code: u16,
        headers: Vec<(String, String)>,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Azure AI Search reply is malformed: {message}")]
    ReplyShape {
        status_code: u16,
        headers: Vec<(String, String)>,
        message: String,
    },
    #[error("the @search.score value is not a decimal number")]
    Score {
        #[source]
        source: DecimalLexemeError,
    },
    #[error("the Azure AI Search attributes could not be encoded")]
    Attributes {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Azure AD token could not be resolved")]
    Token {
        #[source]
        source: AzureTokenError,
    },
    #[error("Azure AI Search vector store creation is not implemented")]
    CreateNotImplemented,
}
