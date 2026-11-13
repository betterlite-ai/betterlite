#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

#[derive(Clone, Copy)]
enum Field {
    BucketName,
    OutputBucketName,
    EncryptionKeyId,
    BucketOwner,
}

enum Expected {
    Value(Field, Option<&'static str>),
    Unrelated(&'static str),
}

fn load(deployment: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let source = format!("model_list:\n- model_name: oracle\n  litellm_params: {{{deployment}}}");
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(&source)
        .map(|config| config.sections.bedrock_services_s3_access)
}

#[rstest]
#[case::loads_s3_bucket_name(
    "s3_bucket_name: configured-bucket",
    &[("AWS_S3_BUCKET_NAME", "env-bucket")],
    Expected::Value(Field::BucketName, Some("configured-bucket"))
)]
#[case::loads_aws_s3_bucket_name(
    "",
    &[("AWS_S3_BUCKET_NAME", "env-bucket")],
    Expected::Value(Field::BucketName, Some("env-bucket"))
)]
#[case::loads_aws_s3_bucket_name_unset("", &[], Expected::Value(Field::BucketName, None))]
#[case::loads_aws_s3_bucket_name_empty(
    "",
    &[("AWS_S3_BUCKET_NAME", "")],
    Expected::Value(Field::BucketName, None)
)]
#[case::loads_s3_bucket_name_reference(
    "s3_bucket_name: os.environ/PROXY_BUCKET",
    &[("PROXY_BUCKET", "referenced-bucket")],
    Expected::Value(Field::BucketName, Some("referenced-bucket"))
)]
#[case::loads_s3_output_bucket_name(
    "s3_output_bucket_name: configured-output",
    &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-output")],
    Expected::Value(Field::OutputBucketName, Some("configured-output"))
)]
#[case::loads_aws_s3_output_bucket_name(
    "",
    &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-output")],
    Expected::Value(Field::OutputBucketName, Some("env-output"))
)]
#[case::loads_aws_s3_output_bucket_name_unset(
    "",
    &[],
    Expected::Value(Field::OutputBucketName, None)
)]
#[case::loads_s3_encryption_key_id(
    "s3_encryption_key_id: configured-key",
    &[("AWS_S3_ENCRYPTION_KEY_ID", "env-key")],
    Expected::Value(Field::EncryptionKeyId, Some("configured-key"))
)]
#[case::loads_aws_s3_encryption_key_id(
    "",
    &[("AWS_S3_ENCRYPTION_KEY_ID", "env-key")],
    Expected::Value(Field::EncryptionKeyId, Some("env-key"))
)]
#[case::loads_aws_s3_encryption_key_id_unset(
    "",
    &[],
    Expected::Value(Field::EncryptionKeyId, None)
)]
#[case::loads_s3_bucket_owner(
    "s3_bucket_owner: '123456789012'",
    &[("AWS_S3_BUCKET_OWNER", "210987654321")],
    Expected::Value(Field::BucketOwner, Some("123456789012"))
)]
#[case::loads_aws_s3_bucket_owner(
    "",
    &[("AWS_S3_BUCKET_OWNER", "210987654321")],
    Expected::Value(Field::BucketOwner, Some("210987654321"))
)]
#[case::loads_aws_s3_bucket_owner_unset("", &[], Expected::Value(Field::BucketOwner, None))]
#[case::rejects_unrelated_key(
    "s3_unrelated_setting: 1",
    &[],
    Expected::Unrelated("model_list.0.litellm_params.s3_unrelated_setting")
)]
fn bedrock_services_s3_access_config_cases(
    #[case] deployment: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    match expected {
        Expected::Value(field, value) => {
            let settings = load(deployment, environment).expect("the config should load");
            let loaded = settings.deployments.first().expect("one deployment");
            let mut fields = (None, None, None, None);
            match field {
                Field::BucketName => fields.0 = value,
                Field::OutputBucketName => fields.1 = value,
                Field::EncryptionKeyId => fields.2 = value,
                Field::BucketOwner => fields.3 = value,
            }
            assert_eq!(
                (
                    loaded.bucket_name.as_deref(),
                    loaded.output_bucket_name.as_deref(),
                    loaded.encryption_key_id.as_deref(),
                    loaded.bucket_owner.as_deref(),
                ),
                fields
            );
        }
        Expected::Unrelated(path) => {
            let error = load(deployment, environment).expect_err("the key is unknown");
            let ConfigError::Unsupported { paths } = error else {
                panic!("expected unsupported keys, got {error:?}");
            };
            assert_eq!(paths, [path.to_owned()]);
        }
    }
}

#[test]
fn a_missing_environment_reference_names_the_variable_and_its_field() {
    let error = load("s3_bucket_name: os.environ/MISSING_BUCKET", &[])
        .expect_err("the reference cannot resolve");
    let rendered = error.to_string();
    assert!(rendered.contains("MISSING_BUCKET"));
    assert!(rendered.contains("model_list.0.litellm_params.s3_bucket_name"));
}
