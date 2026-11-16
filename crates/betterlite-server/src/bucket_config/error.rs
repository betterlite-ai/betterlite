use betterlite_config::sections::includes::IncludeError;

#[derive(Debug, thiserror::Error)]
pub enum BucketConfigError {
    #[error("LITELLM_CONFIG_BUCKET_OBJECT_KEY must be set to load the config from a bucket.")]
    MissingObjectKey,
    #[error("Config object in bucket is not a YAML mapping: {object_key}")]
    NotMapping { object_key: String },
    #[error(
        "Included config could not be read from bucket: {object_key}. The underlying bucket error is logged above."
    )]
    IncludedUnreadable { object_key: String },
    #[error(transparent)]
    Include(#[from] IncludeError),
}
