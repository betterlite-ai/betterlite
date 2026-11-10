use betterlite_context::cloud_storage::CloudStorageError;
use rstest::rstest;

use crate::s3_access::{
    MANAGED_FILE_ID_ERROR, ManagedObjectError, S3_BUCKET_REQUIRED_ERROR, S3AccessError,
    S3AccessParams, TrustedModelCredentials, TrustedValue, allow_legacy_cloud_file_ids,
    configured_bucket_name, configured_bucket_names, decode_managed_file_id, frozen_mapping,
    resolve_managed_s3_object, validate_configured_file_id,
};

use super::support::{assert_bedrock_rejection, environment, params, store};

const BUCKET_NAME_ENV: &str = "AWS_S3_BUCKET_NAME";
const OUTPUT_BUCKET_NAME_ENV: &str = "AWS_S3_OUTPUT_BUCKET_NAME";

fn trusted(bucket: Option<&str>, output: Option<&str>) -> TrustedModelCredentials {
    let mut pairs = Vec::new();
    if let Some(value) = bucket {
        pairs.push(("s3_bucket_name", TrustedValue::Text(value.to_owned())));
    }
    if let Some(value) = output {
        pairs.push((
            "s3_output_bucket_name",
            TrustedValue::Text(value.to_owned()),
        ));
    }
    frozen_mapping(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value)),
    )
}

#[rstest]
#[case::trusted_wins_over_environment(
    Some("trusted-bucket"),
    None,
    &[(BUCKET_NAME_ENV, "env-bucket")],
    Ok("trusted-bucket")
)]
#[case::environment_fills_a_missing_trusted_value(
    None,
    None,
    &[(BUCKET_NAME_ENV, "env-bucket")],
    Ok("env-bucket")
)]
#[case::empty_trusted_value_uses_the_environment(
    Some(""),
    None,
    &[(BUCKET_NAME_ENV, "env-bucket")],
    Ok("env-bucket")
)]
#[case::whitespace_is_a_real_bucket(Some(" "), None, &[(BUCKET_NAME_ENV, "env-bucket")], Ok(" "))]
#[case::empty_environment_counts_as_unset(None, None, &[(BUCKET_NAME_ENV, "")], Err("bucket_required"))]
#[case::nothing_configured(None, None, &[], Err("bucket_required"))]
#[case::output_alone_is_not_enough(None, Some("trusted-output"), &[], Err("bucket_required"))]
fn configured_bucket_name_cases(
    #[case] bucket: Option<&str>,
    #[case] output: Option<&str>,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Result<&str, &str>,
) {
    let result = configured_bucket_name(
        &params(Some(trusted(bucket, output))),
        &environment(variables),
    )
    .map_err(|error| store(&error));
    assert_eq!(result.as_deref().map_err(|error| *error), expected);
}

#[rstest]
#[case::trusted_input_and_output(
    Some("trusted-in"),
    Some("trusted-out"),
    &[],
    Ok(&["trusted-in", "trusted-out"][..])
)]
#[case::input_and_output_from_the_environment(
    None,
    None,
    &[(BUCKET_NAME_ENV, "env-in"), (OUTPUT_BUCKET_NAME_ENV, "env-out")],
    Ok(&["env-in", "env-out"][..])
)]
#[case::output_only_trusted_is_enough(
    Some(""),
    Some("trusted-out"),
    &[(BUCKET_NAME_ENV, "env-in")],
    Ok(&["env-in", "trusted-out"][..])
)]
#[case::duplicates_collapse_in_input_order(
    Some("same"),
    Some("same"),
    &[(BUCKET_NAME_ENV, "ignored"), (OUTPUT_BUCKET_NAME_ENV, "ignored")],
    Ok(&["same"][..])
)]
#[case::environment_duplicates_collapse(
    None,
    None,
    &[(BUCKET_NAME_ENV, "same"), (OUTPUT_BUCKET_NAME_ENV, "same")],
    Ok(&["same"][..])
)]
#[case::empty_environment_values_are_dropped(
    None,
    None,
    &[(BUCKET_NAME_ENV, ""), (OUTPUT_BUCKET_NAME_ENV, "")],
    Err("bucket_required")
)]
#[case::nothing_configured(None, None, &[], Err("bucket_required"))]
fn configured_bucket_names_cases(
    #[case] bucket: Option<&str>,
    #[case] output: Option<&str>,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Result<&[&str], &str>,
) {
    let result = configured_bucket_names(
        &params(Some(trusted(bucket, output))),
        &environment(variables),
    )
    .map_err(|error| store(&error));
    let wanted = expected.map(|entries| {
        entries
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<String>>()
    });
    assert_eq!(result, wanted);
}

#[test]
fn a_missing_bucket_carries_the_litellm_message() {
    let error = configured_bucket_name(&params(None), &environment(&[])).expect_err("no bucket");
    assert_eq!(error.to_string(), S3_BUCKET_REQUIRED_ERROR);
}

#[rstest]
#[case::bucket_name_wrong_type(
    vec![("s3_bucket_name", TrustedValue::Boolean(true))],
    "bucket_name_type"
)]
#[case::output_wrong_type(
    vec![("s3_output_bucket_name", TrustedValue::Other)],
    "output_bucket_name_type"
)]
#[case::null_fields_are_absent(
    vec![
        ("s3_bucket_name", TrustedValue::Null),
        ("s3_output_bucket_name", TrustedValue::Null),
    ],
    "bucket_required"
)]
#[case::unknown_keys_are_ignored(
    vec![("some_other_key", TrustedValue::Text("x".to_owned()))],
    "bucket_required"
)]
fn trusted_snapshot_validation(#[case] pairs: Vec<(&str, TrustedValue)>, #[case] expected: &str) {
    let snapshot = frozen_mapping(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value)),
    );
    let error = configured_bucket_name(&params(Some(snapshot)), &environment(&[]))
        .expect_err("the snapshot has no bucket");
    assert_eq!(store(&error), expected);
}

#[test]
fn request_bucket_fields_do_not_authorize() {
    let request = S3AccessParams {
        trusted: None,
        bucket_name: Some("request-bucket".to_owned()),
        output_bucket_name: Some("request-output".to_owned()),
    };
    let single = configured_bucket_name(&request, &environment(&[]))
        .expect_err("a request field does not authorize");
    assert_eq!(store(&single), "bucket_required");
    let multiple = configured_bucket_names(&request, &environment(&[]))
        .expect_err("a request field does not authorize");
    assert_eq!(store(&multiple), "bucket_required");
    assert_eq!(
        configured_bucket_name(&request, &environment(&[(BUCKET_NAME_ENV, "env-bucket")]))
            .expect("the environment supplies the bucket"),
        "env-bucket"
    );
}

#[test]
fn frozen_mapping_keeps_the_last_duplicate_and_ignores_nested_values() {
    let trusted = frozen_mapping(vec![
        (
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("first".to_owned()),
        ),
        (
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("second".to_owned()),
        ),
        ("s3_output_bucket_name".to_owned(), TrustedValue::Other),
    ]);
    let error = configured_bucket_names(&params(Some(trusted)), &environment(&[]))
        .expect_err("the last output value is not a string");
    assert_eq!(store(&error), "output_bucket_name_type");

    let duplicated = frozen_mapping(vec![
        (
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("first".to_owned()),
        ),
        (
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("second".to_owned()),
        ),
    ]);
    assert_eq!(
        configured_bucket_name(&params(Some(duplicated)), &environment(&[]))
            .expect("the snapshot supplies the bucket"),
        "second"
    );
}

#[rstest]
#[case::default_is_off(None, false)]
#[case::true_boolean(Some(TrustedValue::Boolean(true)), true)]
#[case::false_boolean(Some(TrustedValue::Boolean(false)), false)]
#[case::one(Some(TrustedValue::Text("1".to_owned())), true)]
#[case::true_text(Some(TrustedValue::Text("TRUE".to_owned())), true)]
#[case::trimmed_yes(Some(TrustedValue::Text(" YeS ".to_owned())), true)]
#[case::on(Some(TrustedValue::Text("on".to_owned())), true)]
#[case::zero(Some(TrustedValue::Text("0".to_owned())), false)]
#[case::numeric_one_is_not_text(Some(TrustedValue::Other), false)]
#[case::null_is_off(Some(TrustedValue::Null), false)]
fn legacy_opt_in_reads_only_the_trusted_snapshot(
    #[case] setting: Option<TrustedValue>,
    #[case] expected: bool,
) {
    let trusted = setting
        .map(|value| frozen_mapping(vec![("allow_legacy_cloud_file_ids".to_owned(), value)]));
    assert_eq!(allow_legacy_cloud_file_ids(&params(trusted)), expected);
}

#[test]
fn legacy_mode_relaxes_only_the_managed_name_prefix() {
    let buckets = [String::from("bucket")];
    assert_eq!(
        validate_configured_file_id("s3://bucket/legacy-object", &buckets, true),
        Ok(("bucket".to_owned(), "legacy-object".to_owned()))
    );
    assert_eq!(
        validate_configured_file_id("s3://bucket/legacy-object", &buckets, false),
        Err(CloudStorageError::Unmanaged)
    );
    assert_eq!(
        validate_configured_file_id("s3://bucket/a/../b", &buckets, true),
        Err(CloudStorageError::Segment)
    );
    let prefixed = [String::from("bucket/prefix")];
    assert_eq!(
        validate_configured_file_id("s3://bucket/other/x", &prefixed, true),
        Err(CloudStorageError::PrefixMismatch)
    );
}

#[test]
fn every_bucket_before_the_last_is_tried_and_the_last_error_wins() {
    let buckets = [String::from("other"), String::from("configured/prefix")];
    assert_eq!(
        validate_configured_file_id("s3://configured/litellm-batch-outputs/x", &buckets, false),
        Ok((
            "configured".to_owned(),
            "litellm-batch-outputs/x".to_owned()
        ))
    );
    assert_eq!(
        validate_configured_file_id("s3://configured/plain-object", &buckets, false),
        Err(CloudStorageError::Unmanaged)
    );
}

#[test]
fn a_foreign_file_id_becomes_a_400_bedrock_error() {
    let configured = params(Some(trusted(Some("configured"), None)));
    let rejection = resolve_managed_s3_object(
        "s3://foreign/litellm-bedrock-files-x",
        &configured,
        &environment(&[]),
    )
    .expect_err("the bucket is foreign");
    let ManagedObjectError::Rejected(error) = rejection else {
        panic!("a rejected file id is a BedrockError");
    };
    assert_bedrock_rejection(
        &error,
        "file_id bucket does not match the configured storage bucket",
    );
}

#[test]
fn a_foreign_managed_id_becomes_a_400_bedrock_error() {
    let configured = params(Some(trusted(Some("configured"), None)));
    let rejection = resolve_managed_s3_object("not-a-file-id", &configured, &environment(&[]))
        .expect_err("the id does not decode");
    let ManagedObjectError::Rejected(error) = rejection else {
        panic!("a rejected file id is a BedrockError");
    };
    assert_bedrock_rejection(&error, MANAGED_FILE_ID_ERROR);
}

#[test]
fn missing_configuration_and_snapshot_errors_stay_configuration_errors() {
    let missing = resolve_managed_s3_object("s3://bucket/x", &params(None), &environment(&[]))
        .expect_err("no bucket is configured");
    let ManagedObjectError::Configuration(bucket_error) = missing else {
        panic!("missing configuration is not a BedrockError");
    };
    assert!(matches!(*bucket_error, S3AccessError::BucketRequired));

    let malformed = params(Some(frozen_mapping(vec![(
        "s3_bucket_name".to_owned(),
        TrustedValue::Boolean(false),
    )])));
    let invalid = resolve_managed_s3_object("s3://bucket/x", &malformed, &environment(&[]))
        .expect_err("the snapshot field is not a string");
    let ManagedObjectError::Configuration(type_error) = invalid else {
        panic!("a broken snapshot is not a BedrockError");
    };
    assert!(matches!(*type_error, S3AccessError::BucketNameType));
}

#[test]
fn a_managed_object_resolves_through_the_configured_bucket() {
    let configured = params(Some(trusted(Some("configured"), None)));
    assert_eq!(
        resolve_managed_s3_object(
            "s3://configured/litellm-bedrock-files-x",
            &configured,
            &environment(&[])
        )
        .expect("the managed object resolves"),
        (
            "configured".to_owned(),
            "litellm-bedrock-files-x".to_owned()
        )
    );
    assert_eq!(
        decode_managed_file_id("s3://configured/litellm-bedrock-files-x")
            .expect("a direct URI passes through"),
        "s3://configured/litellm-bedrock-files-x"
    );
}
