#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

#[test]
fn loads_required_bucket_and_defaults_ssl() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n")
    .expect("valid S3 settings");
    assert_eq!(
        config
            .sections
            .s3_response_cache
            .as_ref()
            .unwrap()
            .bucket_name,
        "responses"
    );
    assert_eq!(
        config.sections.s3_response_cache.unwrap().region_name,
        "us-east-1"
    );
}

#[test]
fn resolves_bucket_from_environment() {
    let mut lookup = BTreeMap::new();
    lookup.insert("CACHE_BUCKET".to_owned(), "responses".to_owned());
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  cache_params:\n    s3_bucket_name: os.environ/CACHE_BUCKET\n")
        .expect("environment-backed bucket");
    assert_eq!(
        config.sections.s3_response_cache.unwrap().bucket_name,
        "responses"
    );
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

#[rstest::rstest]
#[case::access_only("s3_aws_access_key_id: access")]
#[case::secret_only("s3_aws_secret_access_key: secret")]
#[case::token_only("s3_aws_session_token: token")]
#[case::insecure("s3_verify: false")]
#[case::api_version("s3_api_version: unsupported")]
fn rejects_unusable_settings(#[case] setting: &str) {
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    {setting}\n"
    );
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap_err();
}

#[test]
fn absent_section_is_none() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    assert!(config.sections.s3_response_cache.is_none());
}

#[rstest::rstest]
#[case::empty("", "")]
#[case::slash("/", "/")]
#[case::prefix("prefix///", "prefix/")]
fn normalizes_prefix(#[case] path: &str, #[case] expected: &str) {
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    s3_path: '{path}'\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    assert_eq!(
        config.sections.s3_response_cache.unwrap().key_prefix,
        expected
    );
}

#[test]
fn resolves_credentials_and_nested_config() {
    use secrecy::ExposeSecret as _;
    let lookup = BTreeMap::from([
        ("ACCESS".to_owned(), "access".to_owned()),
        ("SECRET".to_owned(), "secret".to_owned()),
        ("TOKEN".to_owned(), "token".to_owned()),
    ]);
    let yaml = "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    s3_aws_access_key_id: os.environ/ACCESS\n    s3_aws_secret_access_key: os.environ/SECRET\n    s3_aws_session_token: os.environ/TOKEN\n    s3_config:\n      connect_timeout: 5\n      retries:\n        max_attempts: 3\n      s3:\n        addressing_style: path\n";
    let config = ConfigLoader { lookup }.load(yaml).unwrap();
    let settings = config.sections.s3_response_cache.unwrap();
    let credentials = settings.credentials.unwrap();
    assert_eq!(credentials.access_key.expose_secret(), "access");
    assert_eq!(credentials.secret_key.expose_secret(), "secret");
    assert_eq!(credentials.session_token.unwrap().expose_secret(), "token");
    assert_eq!(settings.config.connect_timeout, 5);
    assert_eq!(settings.config.retries.max_attempts, 3);
    assert!(matches!(
        settings.config.s3.addressing_style,
        super::options::AddressingStyle::Path
    ));
}

#[rstest::rstest]
#[case::ssl("s3_use_ssl: false", "s3_use_ssl")]
#[case::unknown("s3_config: {unknown: 1}", "unknown")]
#[case::unknown_nested("s3_config: {s3: {unknown: 1}}", "unknown")]
#[case::virtual_style("s3_config: {s3: {addressing_style: virtual}}", "addressing_style")]
#[case::huge_timeout("s3_config: {read_timeout: 18446744073709551615}", "read_timeout")]
#[case::zero_timeout("s3_config: {read_timeout: 0}", "read_timeout")]
#[case::zero_connect("s3_config: {connect_timeout: 0}", "connect_timeout")]
#[case::overflow("s3_config: {retries: {max_attempts: 4294967295}}", "max_attempts")]
fn names_rejected_key(#[case] setting: &str, #[case] key: &str) {
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    {setting}\n"
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap_err();
    assert!(error.to_string().contains(key), "{error}");
}

#[rstest::rstest]
#[case::path("s3_path: null")]
#[case::config("s3_config: null")]
fn null_optional_values_use_defaults(#[case] setting: &str) {
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    {setting}\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    let settings = config.sections.s3_response_cache.unwrap();
    assert_eq!(settings.key_prefix, "");
    assert_eq!(settings.config.connect_timeout, 5);
    assert_eq!(settings.config.read_timeout, 30);
}
