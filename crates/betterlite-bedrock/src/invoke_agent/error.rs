use betterlite_http::error::HttpError;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::signing::error::SigningError;

#[derive(Debug, thiserror::Error)]
pub enum InvokeAgentRequestError {
    #[error("Invalid model format. Expected format: 'model=agent/AGENT_ID/ALIAS_ID'")]
    InvalidModelFormat,
    #[error("the request has no messages")]
    EmptyMessages,
    #[error("optional parameter {field} has a type the provider cannot read")]
    WrongType { field: &'static str },
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("the InvokeAgent request parameters could not be serialized")]
    Parameters {
        #[source]
        source: serde_json::Error,
    },
    #[error("the InvokeAgent request could not be signed")]
    Signing {
        #[from]
        source: SigningError,
    },
}
