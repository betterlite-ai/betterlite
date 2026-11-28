use std::num::ParseIntError;
use std::str::Utf8Error;

use betterlite_context::cloud_storage::CloudStorageError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use betterlite_sigv4::error::SigV4Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::request_policy::{Params, RequestValue};
use crate::signing::error::SigningError;

pub mod destination;
pub mod file_object;
pub mod naming;
pub mod signer;
mod tests;

pub const CUSTOM_LLM_PROVIDER: &str = "bedrock";
pub const FILE_UPLOAD_HTTP_METHOD: &str = "PUT";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];
pub const SIGNED_HEADERS_PARAM: &str = "_s3_signed_request_headers";
pub const UPLOAD_URL_PARAM: &str = "upload_url";
pub const UPLOAD_CONTENT_LENGTH_PARAM: &str = "_s3_upload_content_length";

#[derive(Debug, thiserror::Error)]
pub enum S3SigningError {
    #[error("the file content is not valid UTF-8")]
    ContentUtf8(#[source] Utf8Error),
    #[error("a JSONL record of the uploaded file is not valid")]
    Record(#[source] serde_json::Error),
    #[error("a JSONL record must be an object whose body holds a string model")]
    RecordShape,
    #[error(
        "S3 bucket_name is required. Set 's3_bucket_name' in litellm_params or AWS_S3_BUCKET_NAME env var"
    )]
    BucketRequired,
    #[error("file is required")]
    FileRequired,
    #[error("purpose is required")]
    PurposeRequired,
    #[error("{field}: Input should be a valid string")]
    Field {
        field: &'static str,
        #[source]
        source: Option<serde_json::Error>,
    },
    #[error("Unable to parse S3 URL: {url}")]
    UnparsableUrl { url: String },
    #[error("the Content-Length response header is not a whole number")]
    ContentLength(#[source] ParseIntError),
    #[error(transparent)]
    Storage(#[from] CloudStorageError),
    #[error(transparent)]
    File(#[from] MediaMetadataError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(Box<SigningError>),
    #[error("the S3 upload cannot be signed")]
    Sign(#[source] Box<SigV4Error>),
}

// boxed so the two large causes keep every variant within variant_size_differences
impl From<SigningError> for S3SigningError {
    fn from(error: SigningError) -> Self {
        Self::Signing(Box::new(error))
    }
}

impl From<SigV4Error> for S3SigningError {
    fn from(error: SigV4Error) -> Self {
        Self::Sign(Box::new(error))
    }
}

// the signed upload headers B645 hands over are consumed here, so they never travel further with litellm_params
#[must_use]
pub fn validate_environment(headers: &Params, litellm_params: &mut Params) -> Params {
    let mut merged = headers.clone();
    if let Some(RequestValue::Mapping(signed)) = litellm_params.shift_remove(SIGNED_HEADERS_PARAM) {
        merged.extend(signed);
    }
    merged
}

fn param_text(params: &Params, field: &'static str) -> Result<Option<String>, S3SigningError> {
    match params.get(field) {
        None => Ok(None),
        Some(RequestValue::Text(text)) => Ok(Some(text.clone())),
        Some(RequestValue::Raw(raw)) => {
            serde_json::from_str::<Option<String>>(raw).map_err(|source| S3SigningError::Field {
                field,
                source: Some(source),
            })
        }
        Some(RequestValue::Secret(_) | RequestValue::Mapping(_)) => Err(S3SigningError::Field {
            field,
            source: None,
        }),
    }
}
