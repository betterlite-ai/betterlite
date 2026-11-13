use indexmap::IndexMap;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, Error)]
pub enum NativeContentError {
    #[error("the native response is {size} bytes, past the {limit}-byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the native response root is not a JSON object")]
    ResponseNotObject,
    #[error("the native response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the native response could not be transformed")]
    Response {
        status: u16,
        headers: IndexMap<String, String>,
        #[source]
        source: Box<Self>,
    },
    #[error("the generation schema could not be shaped")]
    Schema {
        #[from]
        source: VertexSchemaError,
    },
    #[error("the Gemini API key is missing or not a single string")]
    StudioKey,
    #[error("the Vertex credentials could not be resolved")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the native endpoint URL could not be built")]
    Url {
        #[source]
        source: EndpointTargetError,
    },
}
