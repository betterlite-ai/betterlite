use std::error::Error as StdError;
use std::str;

use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::error::SigningError;

pub const MAX_AWS_ERROR_BODY: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum BedrockIngestionError<TransportFailure>
where
    TransportFailure: StdError,
{
    #[error(
        "No data sources found for Knowledge Base {knowledge_base_id}. Please create a data source first or provide data_source_id and s3_bucket."
    )]
    NoDataSources { knowledge_base_id: String },
    #[error(
        "Could not auto-detect S3 bucket for data source {data_source_id}. Please provide s3_bucket in config."
    )]
    NoS3Bucket { data_source_id: String },
    #[error("a knowledge base id is required to talk to the Bedrock Agent API")]
    MissingKnowledgeBaseId,
    #[error("an S3 bucket is required to create the data source; auto-detection did not find one")]
    MissingS3Bucket,
    #[error("Knowledge Base did not become active in time")]
    KnowledgeBaseTimeout,
    #[error("the AWS {operation} request failed with status {status}: {body}")]
    Aws {
        operation: &'static str,
        status: u16,
        body: String,
    },
    #[error("the AWS {operation} request body could not be serialized")]
    Serialize {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the AWS {operation} response body is not valid JSON")]
    Response {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the AWS credentials could not be resolved")]
    Credentials {
        #[source]
        source: CredentialError,
    },
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("the AWS transport failed")]
    Transport {
        #[source]
        source: TransportFailure,
    },
}

// the first 1024 bytes, and never a header or a credential, reach the message
#[must_use]
pub fn aws_error_body(body: &[u8]) -> String {
    let kept = body.get(..MAX_AWS_ERROR_BODY).unwrap_or(body);
    str::from_utf8(kept).map_or_else(
        |_source| "<a body that is not UTF-8>".to_owned(),
        str::to_owned,
    )
}
