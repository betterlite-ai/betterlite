use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

#[test]
fn loads_required_bucket_and_defaults_ssl() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n")
    .expect("valid S3 settings");
    assert_eq!(config.sections.s3_response_cache.bucket_name, "responses");
    assert!(config.sections.s3_response_cache.use_ssl);
}

#[test]
fn resolves_bucket_from_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert("CACHE_BUCKET".to_owned(), "responses".to_owned());
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache_params:\n    s3_bucket_name: os.environ/CACHE_BUCKET\n")
        .expect("environment-backed bucket");
    assert_eq!(config.sections.s3_response_cache.bucket_name, "responses");
}

#[test]
fn rejects_missing_bucket() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache_params:\n    s3_region_name: eu-west-1\n")
    .expect_err("bucket is required");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}
