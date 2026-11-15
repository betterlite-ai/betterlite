#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    BUCKET_NAME_VARIABLE, BUCKET_OBJECT_KEY_VARIABLE, BUCKET_TYPE_VARIABLE, BucketConfigSource,
    BucketType,
};

const MISSING_KEY: &str =
    "LITELLM_CONFIG_BUCKET_OBJECT_KEY must be set to load the config from a bucket.";

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn from_environment_without_a_bucket_name_is_none() {
    let source = BucketConfigSource::from_environment(&environment(&[])).unwrap();
    assert!(source.is_none());
}

#[test]
fn from_environment_with_an_empty_bucket_name_is_none() {
    let environment = environment(&[
        (BUCKET_NAME_VARIABLE, ""),
        (BUCKET_OBJECT_KEY_VARIABLE, "a"),
    ]);
    let source = BucketConfigSource::from_environment(&environment).unwrap();
    assert!(source.is_none());
}

#[test]
fn from_environment_with_a_bucket_name_and_no_object_key_is_an_error() {
    let environment = environment(&[(BUCKET_NAME_VARIABLE, "bucket")]);
    let error = BucketConfigSource::from_environment(&environment).unwrap_err();
    assert_eq!(error.to_string(), MISSING_KEY);
}

#[test]
fn from_environment_with_a_bucket_name_and_an_empty_object_key_is_an_error() {
    let environment = environment(&[
        (BUCKET_NAME_VARIABLE, "bucket"),
        (BUCKET_OBJECT_KEY_VARIABLE, ""),
    ]);
    let error = BucketConfigSource::from_environment(&environment).unwrap_err();
    assert_eq!(error.to_string(), MISSING_KEY);
}

#[test]
fn from_environment_without_a_type_reads_s3() {
    let environment = environment(&[
        (BUCKET_NAME_VARIABLE, "bucket"),
        (BUCKET_OBJECT_KEY_VARIABLE, "config.yaml"),
    ]);
    let source = BucketConfigSource::from_environment(&environment)
        .unwrap()
        .unwrap();
    assert_eq!(source.bucket_type, BucketType::S3);
    assert_eq!(source.bucket_name, "bucket");
    assert_eq!(source.object_key, "config.yaml");
}

#[rstest]
#[case("gcs", BucketType::Gcs)]
#[case("GCS", BucketType::S3)]
#[case("Gcs", BucketType::S3)]
#[case("s3", BucketType::S3)]
#[case("", BucketType::S3)]
#[case("google", BucketType::S3)]
fn from_environment_reads_the_type_only_from_the_exact_gcs_text(
    #[case] text: &str,
    #[case] expected: BucketType,
) {
    let environment = environment(&[
        (BUCKET_NAME_VARIABLE, "bucket"),
        (BUCKET_OBJECT_KEY_VARIABLE, "config.yaml"),
        (BUCKET_TYPE_VARIABLE, text),
    ]);
    let source = BucketConfigSource::from_environment(&environment)
        .unwrap()
        .unwrap();
    assert_eq!(source.bucket_type, expected);
}

#[test]
fn from_environment_keeps_the_names_verbatim() {
    let environment = environment(&[
        (BUCKET_NAME_VARIABLE, " team-bucket "),
        (BUCKET_OBJECT_KEY_VARIABLE, " team/config.yaml "),
    ]);
    let source = BucketConfigSource::from_environment(&environment)
        .unwrap()
        .unwrap();
    assert_eq!(source.bucket_name, " team-bucket ");
    assert_eq!(source.object_key, " team/config.yaml ");
}
