use betterlite_config::loader::EnvironmentLookup;

use super::error::BucketConfigError;

pub const BUCKET_NAME_VARIABLE: &str = "LITELLM_CONFIG_BUCKET_NAME";
pub const BUCKET_OBJECT_KEY_VARIABLE: &str = "LITELLM_CONFIG_BUCKET_OBJECT_KEY";
pub const BUCKET_TYPE_VARIABLE: &str = "LITELLM_CONFIG_BUCKET_TYPE";
const GCS_TYPE_TEXT: &str = "gcs";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketType {
    S3,
    Gcs,
}

impl BucketType {
    // LiteLLM reads GCS only for the exact text "gcs"; unset, "GCS", "s3" and anything else are S3
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        if text == GCS_TYPE_TEXT {
            Self::Gcs
        } else {
            Self::S3
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketConfigSource {
    pub bucket_type: BucketType,
    pub bucket_name: String,
    pub object_key: String,
}

impl BucketConfigSource {
    pub fn from_environment<L>(environment: &L) -> Result<Option<Self>, BucketConfigError>
    where
        L: EnvironmentLookup + ?Sized,
    {
        let Some(bucket_name) = present(environment, BUCKET_NAME_VARIABLE) else {
            return Ok(None);
        };
        let Some(object_key) = present(environment, BUCKET_OBJECT_KEY_VARIABLE) else {
            return Err(BucketConfigError::MissingObjectKey);
        };
        let bucket_type = environment
            .lookup(BUCKET_TYPE_VARIABLE)
            .map_or(BucketType::S3, |text| BucketType::from_text(&text));
        Ok(Some(Self {
            bucket_type,
            bucket_name,
            object_key,
        }))
    }
}

fn present<L>(environment: &L, name: &str) -> Option<String>
where
    L: EnvironmentLookup + ?Sized,
{
    environment.lookup(name).filter(|value| !value.is_empty())
}

mod tests;
