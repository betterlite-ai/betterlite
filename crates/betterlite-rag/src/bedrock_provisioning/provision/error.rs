use std::error::Error as StdError;

use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_provisioning::calls::error::CallError;

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError<TransportFailure>
where
    TransportFailure: StdError,
{
    #[error(transparent)]
    Call(#[from] CallError),
    #[error(transparent)]
    Ingestion(#[from] BedrockIngestionError<TransportFailure>),
    #[error("the OpenSearch collection did not become active in time")]
    CollectionTimeout,
    #[error(
        "the OpenSearch index was not created after {attempts} attempts; the data access policy may not have propagated: {status} {body}"
    )]
    IndexRetriesExhausted {
        attempts: usize,
        status: u16,
        body: String,
    },
}
