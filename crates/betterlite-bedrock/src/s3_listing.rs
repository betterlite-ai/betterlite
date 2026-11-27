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
pub mod objects;
pub mod prefix;
pub mod requests;
pub mod responses;
pub mod sign;
pub mod target;
mod tests;

pub use decode::{DecodeLimits, S3ListedEntry, S3Listing, decode_s3_listing};

// the internal keys the listing walk hands from one request transform to the next
pub const LIST_FILES_PURPOSE_PARAM: &str = "_s3_list_files_purpose";
pub const LIST_FILES_LOCATION_PARAM: &str = "_s3_list_files_location";
// Python's mapping for a file id names the raw bucket; the scheme is rebuilt here
pub const S3_SCHEME: &str = "s3://";

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
        match self {
            Self::Bedrock(error) => Some(error.error.status_code),
            Self::Managed(error) => error.status_code(),
            Self::Access(_)
            | Self::Storage(_)
            | Self::Region(_)
            | Self::Credential(_)
            | Self::Signing(_)
            | Self::Header(_)
            | Self::Field { .. }
            | Self::Size { .. }
            | Self::Timestamp { .. }
            | Self::FileIdRequired
            | Self::RetrievalUnsupported
            | Self::DeleteContext
            | Self::BodyTooLarge { .. }
            | Self::DepthExceeded { .. }
            | Self::Dtd
            | Self::UnboundPrefix { .. }
            | Self::Entity { .. }
            | Self::NoRoot
            | Self::Unclosed
            | Self::Trailing
            | Self::Xml(_)
            | Self::Text(_) => None,
        }
    }
}
