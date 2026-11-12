use super::error::GcsAccessError;

pub const GCS_BATCH_BUCKET_ENV: &str = "GCS_BATCH_BUCKET_NAME";
pub const GCS_BUCKET_ENV: &str = "GCS_BUCKET_NAME";

// LiteLLM tries the deployment spellings first, then the batch and general environment variables
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BucketCandidates {
    pub deployment: Option<String>,
    pub deployment_alias: Option<String>,
    pub batch_environment: Option<String>,
    pub environment: Option<String>,
}

pub fn configured_bucket_name(candidates: &BucketCandidates) -> Result<String, GcsAccessError> {
    [
        candidates.deployment.as_deref(),
        candidates.deployment_alias.as_deref(),
        candidates.batch_environment.as_deref(),
        candidates.environment.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|bucket| !bucket.is_empty())
    .map(str::to_owned)
    .ok_or(GcsAccessError::BucketRequired)
}
