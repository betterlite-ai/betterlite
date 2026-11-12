#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::bedrock_ingestion::settings::{
    BedrockRagSettings, DEFAULT_EMBEDDING_MODEL, DEFAULT_INGESTION_TIMEOUT, SettingsError,
};
use crate::bedrock_ingestion::tests::support::{settings, settings_in, vector_config};

fn parse(text: &str) -> Result<BedrockRagSettings, SettingsError> {
    let config = vector_config(text);
    BedrockRagSettings::from_vector_store_config(&config, &BTreeMap::new())
}

fn environment(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

#[test]
fn settings_vector_store_id_wins_over_knowledge_base_id() {
    let parsed = settings(
        r#"{"vector_store_id": "KB-FROM-VECTOR-STORE", "knowledge_base_id": "KB-LEGACY"}"#,
    );
    assert_eq!(
        parsed.knowledge_base_id.as_deref(),
        Some("KB-FROM-VECTOR-STORE")
    );
}

#[test]
fn settings_falls_back_to_knowledge_base_id_when_the_vector_store_id_is_empty() {
    let parsed = settings(r#"{"vector_store_id": "", "knowledge_base_id": "KB-LEGACY"}"#);
    assert_eq!(parsed.knowledge_base_id.as_deref(), Some("KB-LEGACY"));
}

#[test]
fn settings_empty_ids_and_strings_count_as_absent() {
    let parsed = settings(
        r#"{"vector_store_id": "", "knowledge_base_id": null, "data_source_id": "",
            "s3_bucket": "", "s3_prefix": ""}"#,
    );
    assert!(parsed.knowledge_base_id.is_none());
    assert!(parsed.data_source_id.is_none());
    assert!(parsed.s3_bucket.is_none());
    assert!(parsed.s3_prefix.is_none());
}

#[test]
fn settings_defaults_apply_to_absent_values() {
    let parsed = settings("{}");
    assert_eq!(parsed.embedding_model, DEFAULT_EMBEDDING_MODEL);
    assert!(!parsed.wait_for_ingestion);
    assert_eq!(parsed.ingestion_timeout, DEFAULT_INGESTION_TIMEOUT);
    assert_eq!(parsed.aws_region_name, "us-west-2");
    assert!(parsed.s3_prefix.is_none());
    assert!(parsed.credentials.access_key_id.is_none());
}

#[test]
fn settings_keeps_explicit_values() {
    let parsed = settings(
        r#"{"embedding_model": "cohere.embed-english-v3", "wait_for_ingestion": true,
            "s3_prefix": "custom/", "s3_bucket": "bucket-name", "data_source_id": "DS1"}"#,
    );
    assert_eq!(parsed.embedding_model, "cohere.embed-english-v3");
    assert!(parsed.wait_for_ingestion);
    assert_eq!(parsed.s3_prefix.as_deref(), Some("custom/"));
    assert_eq!(parsed.s3_bucket.as_deref(), Some("bucket-name"));
    assert_eq!(parsed.data_source_id.as_deref(), Some("DS1"));
}

#[test]
fn settings_empty_embedding_model_takes_the_default() {
    let parsed = settings(r#"{"embedding_model": ""}"#);
    assert_eq!(parsed.embedding_model, DEFAULT_EMBEDDING_MODEL);
}

#[rstest]
#[case::null("null", DEFAULT_INGESTION_TIMEOUT)]
#[case::integer("3000", 3000)]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
#[case::float_truncates_toward_zero("12.9", 12)]
#[case::negative_float_truncates_toward_zero("-12.9", -12)]
#[case::numeric_string(r#""600""#, 600)]
#[case::padded_numeric_string(r#"" 42 ""#, 42)]
fn settings_ingestion_timeout_accepts_its_types(#[case] value: &str, #[case] expected: i64) {
    let text = format!(r#"{{"ingestion_timeout": {value}}}"#);
    let parsed = settings(&text);
    assert_eq!(parsed.ingestion_timeout, expected);
}

#[rstest]
#[case::boolean("true")]
#[case::list("[1, 2]")]
#[case::object(r#"{"seconds": 30}"#)]
#[case::text(r#""soon""#)]
#[case::fractional_text(r#""12.5""#)]
#[case::overflowing_float("1e300")]
fn settings_ingestion_timeout_rejects_other_values(#[case] value: &str) {
    let text = format!(r#"{{"ingestion_timeout": {value}}}"#);
    let error = parse(&text).expect_err("the timeout is not an integer");
    assert!(matches!(error, SettingsError::Timeout), "{error}");
}

#[test]
fn settings_region_takes_the_explicit_value() {
    let parsed = settings(r#"{"aws_region_name": "eu-central-1"}"#);
    assert_eq!(parsed.aws_region_name, "eu-central-1");
}

#[test]
fn settings_empty_explicit_region_falls_through_to_the_environment() {
    let parsed = settings_in(
        r#"{"aws_region_name": ""}"#,
        &environment("AWS_REGION_NAME", "ap-south-1"),
    );
    assert_eq!(parsed.aws_region_name, "ap-south-1");
}

#[test]
fn settings_aws_region_wins_over_aws_region_name() {
    let parsed = settings_in(
        "{}",
        &BTreeMap::from([
            ("AWS_REGION_NAME".to_owned(), "ap-south-1".to_owned()),
            ("AWS_REGION".to_owned(), "ap-northeast-1".to_owned()),
        ]),
    );
    assert_eq!(parsed.aws_region_name, "ap-northeast-1");
}

#[test]
fn settings_invalid_explicit_region_is_an_error() {
    let error = parse(r#"{"aws_region_name": "US-EAST-1"}"#).expect_err("the region is invalid");
    assert!(matches!(error, SettingsError::Region(_)), "{error}");
}

#[test]
fn settings_empty_environment_region_is_an_error() {
    let config = vector_config("{}");
    let error =
        BedrockRagSettings::from_vector_store_config(&config, &environment("AWS_REGION", ""))
            .expect_err("an empty AWS_REGION does not name a region");
    assert!(matches!(error, SettingsError::Region(_)), "{error}");
}

const CREDENTIAL_CONFIG: &str = r#"{"aws_access_key_id": "AKID",
    "aws_secret_access_key": "SECRET", "aws_session_token": "TOKEN",
    "aws_session_name": "session", "aws_profile_name": "profile",
    "aws_role_name": "arn:aws:iam::123456789012:role/Role",
    "aws_web_identity_token": "identity", "aws_sts_endpoint": "https://sts.example.com",
    "aws_external_id": "external"}"#;

#[test]
fn settings_credentials_keep_secret_keys() {
    let parsed = settings(CREDENTIAL_CONFIG);
    let credentials = &parsed.credentials;
    assert_eq!(
        credentials
            .access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("AKID")
    );
    assert_eq!(
        credentials
            .secret_access_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("SECRET")
    );
    assert_eq!(
        credentials
            .session_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("TOKEN")
    );
    assert_eq!(
        credentials
            .web_identity_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("identity")
    );
}

#[test]
fn settings_credentials_keep_plain_keys() {
    let parsed = settings(CREDENTIAL_CONFIG);
    let credentials = &parsed.credentials;
    assert_eq!(credentials.session_name.as_deref(), Some("session"));
    assert_eq!(credentials.profile_name.as_deref(), Some("profile"));
    assert_eq!(
        credentials.role_name.as_deref(),
        Some("arn:aws:iam::123456789012:role/Role")
    );
    assert_eq!(
        credentials.sts_endpoint.as_deref(),
        Some("https://sts.example.com")
    );
    assert_eq!(credentials.external_id.as_deref(), Some("external"));
}

#[test]
fn settings_credentials_keep_empty_strings() {
    let parsed = settings(r#"{"aws_external_id": "", "aws_session_name": ""}"#);
    assert_eq!(parsed.credentials.external_id.as_deref(), Some(""));
    assert_eq!(parsed.credentials.session_name.as_deref(), Some(""));
}

#[test]
fn settings_wrong_type_names_the_field() {
    let bucket_error = parse(r#"{"s3_bucket": 5}"#).expect_err("a number is not a bucket name");
    assert_eq!(bucket_error.to_string(), "s3_bucket must be a string");
    let flag_error = parse(r#"{"wait_for_ingestion": "yes"}"#).expect_err("a string is not a flag");
    assert_eq!(
        flag_error.to_string(),
        "wait_for_ingestion must be a string"
    );
}
