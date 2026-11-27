use betterlite_context::cloud_storage::CloudStorageError;
use betterlite_logging::s3_transport::error::S3TransportError;
use quick_xml::encoding::EncodingError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::credentials::error::CredentialError;
use crate::s3_access::S3AccessError;
use crate::s3_access::managed::ManagedObjectError;

pub mod decode;
pub mod prefix;
pub mod requests;
pub mod responses;
pub mod target;
mod tests;

// the internal keys the listing walk hands from one request transform to the next
pub const LIST_FILES_PURPOSE_PARAM: &str = "_s3_list_files_purpose";
pub const LIST_FILES_LOCATION_PARAM: &str = "_s3_list_files_location";

#[derive(Debug, Error)]
pub enum S3ListingError {
    #[error(transparent)]
    Access(#[from] S3AccessError),
    #[error(transparent)]
    Storage(#[from] CloudStorageError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Signing(#[from] S3TransportError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Managed(#[from] ManagedObjectError),
    #[error("bedrock provider error")]
    Bedrock(Box<BedrockError>),
    #[error("{field}: Input should be a valid string")]
    Field {
        field: &'static str,
        #[source]
        source: Option<serde_json::Error>,
    },
    #[error("invalid S3 listing size {text:?}")]
    Size { text: String },
    #[error("invalid S3 listing timestamp {text:?}")]
    Timestamp { text: String },
    #[error("file_id is required for Bedrock file content retrieval")]
    FileIdRequired,
    #[error("BedrockFilesConfig does not support file retrieval")]
    RetrievalUnsupported,
    #[error("additional_args must carry a nonempty file_id string")]
    DeleteContext,
    #[error("the S3 listing response is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("the S3 listing XML nests deeper than {limit} elements")]
    DepthExceeded { limit: usize },
    #[error("the S3 listing XML declares a document type")]
    Dtd,
    #[error("the S3 listing XML uses the undeclared namespace prefix {prefix}")]
    UnboundPrefix { prefix: String },
    #[error("the S3 listing XML references the undefined entity {name}")]
    Entity { name: String },
    #[error("the S3 listing XML has no root element")]
    NoRoot,
    #[error("the S3 listing XML ends before its root element closes")]
    Unclosed,
    #[error("the S3 listing XML has content after its root element")]
    Trailing,
    #[error(transparent)]
    Xml(#[from] quick_xml::Error),
    #[error("the S3 listing XML text is not valid UTF-8")]
    Text(#[from] EncodingError),
}

impl S3ListingError {
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        if let Self::Bedrock(error) = self {
            return Some(error.error.status_code);
        }
        if let Self::Managed(error) = self {
            return Some(error.status_code());
        }
        None
    }
}
