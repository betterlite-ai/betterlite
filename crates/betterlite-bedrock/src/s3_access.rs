use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_services_s3_access::{
    BUCKET_NAME_ENV, OUTPUT_BUCKET_NAME_ENV,
};
use betterlite_context::cloud_storage::CloudStorageError;
use betterlite_context::cloud_storage::policy::{
    LegacyCloudSetting, TrustedCloudCredentials, allow_legacy,
};
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;

pub mod decode;
pub mod handler;
pub mod managed;
pub mod models;
pub mod settings;
mod tests;
pub mod uri;

pub const S3_BUCKET_REQUIRED_ERROR: &str = "S3 bucket_name is required. Set 's3_bucket_name' in proxy config or AWS_S3_BUCKET_NAME for Bedrock file content retrieval.";
pub const MANAGED_FILE_ID_ERROR: &str = "file_id must be a managed LiteLLM S3 file id";
pub const REJECTED_FILE_ID_REQUEST_URL: &str = "https://litellm.ai";
pub const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";
pub const MANAGED_FILE_ID_MARKER: &str = "llm_output_file_id,";

const S3_SCHEME: &str = "s3://";
const BUCKET_NAME_PARAM: &str = "s3_bucket_name";
const OUTPUT_BUCKET_NAME_PARAM: &str = "s3_output_bucket_name";
const LEGACY_SETTING: &str = "allow_legacy_cloud_file_ids";

#[derive(Debug, thiserror::Error)]
pub enum S3AccessError {
    #[error("file_id must be a managed LiteLLM S3 file id")]
    ManagedFileId,
    #[error(
        "S3 bucket_name is required. Set 's3_bucket_name' in proxy config or AWS_S3_BUCKET_NAME for Bedrock file content retrieval."
    )]
    BucketRequired,
    #[error("s3_bucket_name: Input should be a valid string")]
    BucketNameType,
    #[error("s3_output_bucket_name: Input should be a valid string")]
    OutputBucketNameType,
    #[error("Invalid S3 URI format: {uri}")]
    InvalidS3Uri { uri: String },
    #[error(transparent)]
    Storage(#[from] CloudStorageError),
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedValue {
    Text(String),
    Boolean(bool),
    Null,
    Other,
}

// The immutable copy of _litellm_internal_model_credentials; the private map is only built by frozen_mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedModelCredentials {
    values: BTreeMap<String, TrustedValue>,
}

#[must_use]
pub fn frozen_mapping<I>(items: I) -> TrustedModelCredentials
where
    I: IntoIterator<Item = (String, TrustedValue)>,
{
    let mut values = BTreeMap::new();
    for (name, value) in items {
        values.insert(name, value);
    }
    TrustedModelCredentials { values }
}

impl TrustedModelCredentials {
    fn text(&self, name: &str, wrong_type: S3AccessError) -> Result<Option<String>, S3AccessError> {
        match self.values.get(name) {
            None | Some(TrustedValue::Null) => Ok(None),
            Some(TrustedValue::Text(value)) => Ok(Some(value.clone())),
            Some(TrustedValue::Boolean(_) | TrustedValue::Other) => Err(wrong_type),
        }
    }

    fn bucket_fields(&self) -> Result<(Option<String>, Option<String>), S3AccessError> {
        let bucket = self.text(BUCKET_NAME_PARAM, S3AccessError::BucketNameType)?;
        let output = self.text(
            OUTPUT_BUCKET_NAME_PARAM,
            S3AccessError::OutputBucketNameType,
        )?;
        Ok((bucket, output))
    }

    #[must_use]
    pub fn allows_legacy(&self) -> bool {
        let setting = match self.values.get(LEGACY_SETTING) {
            Some(TrustedValue::Boolean(flag)) => LegacyCloudSetting::Boolean(*flag),
            Some(TrustedValue::Text(value)) => LegacyCloudSetting::Text(value.clone()),
            Some(TrustedValue::Null | TrustedValue::Other) | None => LegacyCloudSetting::Other,
        };
        allow_legacy(Some(&TrustedCloudCredentials::from(setting)))
    }
}

#[derive(Debug, Clone, Default)]
pub struct S3AccessParams {
    pub trusted: TrustedModelCredentials,
    // a deployment or request bucket name carried in litellm_params never authorizes a file id on its own
    pub bucket_name: Option<String>,
    pub output_bucket_name: Option<String>,
}

fn bucket_or_environment(
    trusted: Option<String>,
    environment: &BTreeMap<String, String>,
    variable: &str,
) -> Option<String> {
    trusted.filter(|bucket| !bucket.is_empty()).or_else(|| {
        environment
            .get(variable)
            .filter(|bucket| !bucket.is_empty())
            .cloned()
    })
}

pub fn configured_bucket_name(
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<String, S3AccessError> {
    let (input, _output) = params.trusted.bucket_fields()?;
    bucket_or_environment(input, environment, BUCKET_NAME_ENV).ok_or(S3AccessError::BucketRequired)
}

pub fn configured_bucket_names(
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<Vec<String>, S3AccessError> {
    let (input, output) = params.trusted.bucket_fields()?;
    let candidates = [(input, BUCKET_NAME_ENV), (output, OUTPUT_BUCKET_NAME_ENV)];
    let mut buckets = Vec::new();
    for (trusted, variable) in candidates {
        if let Some(bucket) = bucket_or_environment(trusted, environment, variable)
            && !buckets.contains(&bucket)
        {
            buckets.push(bucket);
        }
    }
    if buckets.is_empty() {
        return Err(S3AccessError::BucketRequired);
    }
    Ok(buckets)
}
