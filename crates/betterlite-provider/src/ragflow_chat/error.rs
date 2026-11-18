use betterlite_http::error::HttpError;
use betterlite_openai::chat_content::ChatContentError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RagflowError {
    // the three ValueErrors _parse_ragflow_model raises
    #[error(
        "Invalid RAGFlow model format: {model}. Expected format: ragflow/chat/{{chat_id}}/{{model}} or ragflow/agent/{{agent_id}}/{{model}}"
    )]
    ModelTooShort { model: String },
    #[error("Invalid RAGFlow model format: {model}. Must start with 'ragflow/'")]
    ModelWrongPrefix { model: String },
    #[error("Invalid RAGFlow endpoint type: {endpoint_type}. Must be 'chat' or 'agent'")]
    EndpointType { endpoint_type: String },
    // the ValueError get_complete_url raises when no base source resolves
    #[error(
        "api_base is required for RAGFlow provider. Set it via api_base parameter, RAGFLOW_API_BASE environment variable, or litellm.api_base"
    )]
    ApiBaseRequired,
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    // encode_url_path_segment's ValueError text, which HttpError carries unchanged
    #[error(transparent)]
    EntityId(#[from] HttpError),
    #[error("a chat message could not be converted for the request body: {source}")]
    MessageShape {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
}
