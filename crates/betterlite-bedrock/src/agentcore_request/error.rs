use crate::signing::error::SigningError;

#[derive(Debug, thiserror::Error)]
pub enum AgentCoreRequestError {
    #[error(
        "Invalid model format. Expected format: 'model=bedrock/agentcore/arn:aws:bedrock-agentcore:region:account:runtime/runtime_id'"
    )]
    InvalidModelFormat,
    #[error("Invalid ARN format: {arn}")]
    InvalidArn { arn: String },
    #[error("the request has no messages")]
    EmptyMessages,
    #[error("optional parameter {field} has a type the provider cannot read")]
    WrongType { field: &'static str },
    #[error("the AgentCore request parameters could not be serialized")]
    Parameters {
        #[source]
        source: serde_json::Error,
    },
    #[error("the AgentCore request could not be signed")]
    Signing {
        #[from]
        source: SigningError,
    },
}
