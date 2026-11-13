use betterlite_http::error::HttpError;
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

use crate::api::responses_provider_contract::error::ResponsesContractError;

pub const MISSING_API_KEY_MESSAGE: &str =
    "Manus API key is required. Set MANUS_API_KEY environment variable or pass api_key parameter.";

#[derive(Debug, Error)]
pub enum ManusResponsesError {
    #[error(transparent)]
    Contract(#[from] ResponsesContractError),
    #[error(
        "Manus API key is required. Set MANUS_API_KEY environment variable or pass api_key parameter."
    )]
    MissingApiKey,
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    #[error("the {field} parameter must be text")]
    ParamNotText { field: &'static str },
    #[error("the input parameter must be text or a list of items")]
    InputShape,
    #[error("the extra_body parameter must be an object")]
    ExtraBodyShape,
    #[error("the response id {response_id} cannot be written as a URL path segment: {source}")]
    ResponseId {
        response_id: String,
        #[source]
        source: HttpError,
    },
    #[error(transparent)]
    Request(#[from] ResponseInputError),
    #[error(transparent)]
    Resource(#[from] ResponseResourceError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the request parameters are not an object")]
    ParamsShape,
    #[error("a Responses stream chunk must be a JSON object")]
    StreamChunkShape,
    #[error("a Responses stream chunk has no type field")]
    StreamChunkWithoutType,
    #[error("the input items reply is not a JSON object")]
    InputItemsShape,
}
