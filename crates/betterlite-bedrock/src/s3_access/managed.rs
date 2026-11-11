use std::collections::BTreeMap;

use betterlite_context::cloud_storage::{
    BEDROCK_MANAGED_S3_PREFIXES, CloudStorageError, decode_cloud_object_reference,
};
use betterlite_openai::error::{ErrorRequest, ErrorResponse, ProviderError, ProviderErrorParts};

use super::decode::decode_managed_file_id;
use super::{
    REJECTED_FILE_ID_REQUEST_URL, S3_SCHEME, S3AccessError, S3AccessParams, configured_bucket_names,
};
use crate::bedrock_settings::{BedrockError, ResponseBody};

pub fn validate_configured_file_id(
    s3_uri: &str,
    configured_bucket_names: &[String],
    allow_legacy_cloud_file_ids: bool,
) -> Result<(String, String), CloudStorageError> {
    let Some((last, rest)) = configured_bucket_names.split_last() else {
        return Err(CloudStorageError::EmptyBucket);
    };
    for bucket in rest {
        if let Ok(found) = decode_cloud_object_reference(
            s3_uri,
            S3_SCHEME,
            bucket,
            &BEDROCK_MANAGED_S3_PREFIXES,
            allow_legacy_cloud_file_ids,
        ) {
            return Ok(found);
        }
    }
    decode_cloud_object_reference(
        s3_uri,
        S3_SCHEME,
        last,
        &BEDROCK_MANAGED_S3_PREFIXES,
        allow_legacy_cloud_file_ids,
    )
}

// The Python BedrockError carries the rejected message as its response text and a synthetic GET request.
#[must_use]
pub fn rejected_file_id(reason: &S3AccessError) -> BedrockError {
    let message = reason.to_string();
    let parts = ProviderErrorParts {
        status_code: 400,
        message: message.clone(),
        headers: None,
        request: None,
        response: Some(ErrorResponse {
            status_code: 400,
            request: ErrorRequest {
                method: "GET".to_owned(),
                url: REJECTED_FILE_ID_REQUEST_URL.to_owned(),
            },
        }),
        body: None,
        status_code_is_synthesized: false,
    };
    BedrockError {
        error: ProviderError::from(parts),
        headers: None,
        response_headers: Vec::new(),
        response_body: ResponseBody::Read(message),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManagedObjectError {
    #[error(transparent)]
    Configuration(Box<S3AccessError>),
    #[error("{}", .0.error.message)]
    Rejected(Box<BedrockError>),
}

impl ManagedObjectError {
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Configuration(_) => None,
            Self::Rejected(error) => Some(error.error.status_code),
        }
    }
}

pub fn resolve_managed_s3_object(
    file_id: &str,
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<(String, String), ManagedObjectError> {
    let buckets = configured_bucket_names(params, environment)
        .map_err(|error| ManagedObjectError::Configuration(Box::new(error)))?;
    let legacy = params.trusted.allows_legacy();
    let s3_uri = decode_managed_file_id(file_id)
        .map_err(|error| ManagedObjectError::Rejected(Box::new(rejected_file_id(&error))))?;
    validate_configured_file_id(&s3_uri, &buckets, legacy).map_err(|error| {
        let reason = S3AccessError::Storage(error);
        ManagedObjectError::Rejected(Box::new(rejected_file_id(&reason)))
    })
}
