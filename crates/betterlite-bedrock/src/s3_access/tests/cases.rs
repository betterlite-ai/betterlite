use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_context::cloud_storage::CloudStorageError;
use secrecy::ExposeSecret as _;

use crate::aws_settings::AUTHENTICATION_PARAMS;
use crate::s3_access::decode::decode_managed_file_id;
use crate::s3_access::handler::{AwsBase, BatchFiles, BedrockFilesHandler};
use crate::s3_access::managed::{
    ManagedObjectError, resolve_managed_s3_object, validate_configured_file_id,
};
use crate::s3_access::models::bedrock_model_id;
use crate::s3_access::settings::{S3Setting, S3SettingSources, resolve_s3_setting};
use crate::s3_access::uri::parse_s3_uri;
use crate::s3_access::{
    S3AccessError, S3AccessParams, TrustedModelCredentials, TrustedValue, configured_bucket_name,
    configured_bucket_names, frozen_mapping,
};

use super::fixtures::{
    MANAGED_FILE_ID, MANAGED_OBJECT, MARKER_SEMICOLON_FILE_ID, MARKER_TWICE_FILE_ID,
    UNPADDED_FILE_ID, UNPADDED_OBJECT,
};
use super::model_helpers::{invoke_with_empty_context, strip_with_empty_context};
use super::support::{
    assert_bedrock_rejection, client, credential_cache, environment, params, secrets, settings,
    snapshot, store,
};

#[rstest::rstest]
#[case::managed_file_id_decode(managed_file_id_decode)]
#[case::configured_input_output_buckets(configured_input_output_buckets)]
#[case::configured_prefix_boundary(configured_prefix_boundary)]
#[case::foreign_bucket_rejected(foreign_bucket_rejected)]
#[case::legacy_id_opt_in(legacy_id_opt_in)]
#[case::request_credentials_rejected(request_credentials_rejected)]
#[case::kms_owner_precedence(kms_owner_precedence)]
#[case::s3_model_helpers(s3_model_helpers)]
#[case::plain_s3_uri_parser(plain_s3_uri_parser)]
#[case::files_handler_initialization(files_handler_initialization)]
fn s3_access_cases(#[case] case: fn()) {
    case();
}

const BUCKET_NAME_ENV: &str = "AWS_S3_BUCKET_NAME";
const OUTPUT_BUCKET_NAME_ENV: &str = "AWS_S3_OUTPUT_BUCKET_NAME";

fn managed_file_id_decode() {
    assert_eq!(
        decode_managed_file_id(MANAGED_FILE_ID).expect("the managed id decodes"),
        MANAGED_OBJECT
    );
    assert_eq!(
        decode_managed_file_id(UNPADDED_FILE_ID).expect("an unpadded id decodes"),
        UNPADDED_OBJECT
    );
    assert_eq!(
        decode_managed_file_id("s3://direct/litellm-bedrock-files-key")
            .expect("a raw s3 URI passes through"),
        "s3://direct/litellm-bedrock-files-key"
    );
    assert_eq!(
        decode_managed_file_id(MARKER_TWICE_FILE_ID).expect("the first marker wins"),
        "first-value"
    );
    assert_eq!(
        decode_managed_file_id(MARKER_SEMICOLON_FILE_ID).expect("the semicolon ends the value"),
        "value-before"
    );
    let encoded = decode_managed_file_id("s3%3A%2F%2Fbucket%2Fkey")
        .expect_err("a percent-encoded scheme is not s3://");
    assert_eq!(store(&encoded), "managed_file_id");
    let plain = decode_managed_file_id("not-a-file-id").expect_err("plain text is not an id");
    assert_eq!(store(&plain), "managed_file_id");
}

fn configured_input_output_buckets() {
    let output_only = params(snapshot(&[(
        "s3_output_bucket_name",
        TrustedValue::Text("trusted-out".to_owned()),
    )]));
    let request = S3AccessParams {
        trusted: output_only.trusted,
        bucket_name: Some("request-in".to_owned()),
        output_bucket_name: Some("request-out".to_owned()),
    };
    let variables = environment(&[
        (BUCKET_NAME_ENV, "env-in"),
        (OUTPUT_BUCKET_NAME_ENV, "env-out"),
    ]);
    assert_eq!(
        configured_bucket_name(&request, &variables).expect("the environment supplies input"),
        "env-in"
    );
    assert_eq!(
        configured_bucket_names(&request, &variables).expect("both buckets resolve"),
        ["env-in", "trusted-out"]
    );
    let only_output = params(snapshot(&[(
        "s3_output_bucket_name",
        TrustedValue::Text("trusted-out".to_owned()),
    )]));
    let empty_variables = environment(&[]);
    let single = configured_bucket_name(&only_output, &empty_variables)
        .expect_err("an output bucket is not an input bucket");
    assert!(matches!(single, S3AccessError::BucketRequired));
    assert_eq!(
        configured_bucket_names(&only_output, &empty_variables).expect("output alone suffices"),
        ["trusted-out"]
    );
}

fn configured_prefix_boundary() {
    let buckets = [String::from("bucket")];
    assert_managed_prefixes(&buckets);
    assert_prefix_boundaries(&buckets);
    assert_one_pass_decoding(&buckets);
}

fn assert_managed_prefixes(buckets: &[String]) {
    for uri in [
        "s3://bucket/litellm-bedrock-files-x",
        "s3://bucket/litellm-bedrock-files/x",
        "s3://bucket/litellm-batch-outputs/x",
    ] {
        validate_configured_file_id(uri, buckets, false).expect("the managed prefix is accepted");
    }
    assert_eq!(
        validate_configured_file_id("s3://bucket/litellm-bedrock-file/x", buckets, false),
        Err(CloudStorageError::Unmanaged)
    );
}

fn assert_prefix_boundaries(buckets: &[String]) {
    assert_eq!(
        validate_configured_file_id("s3://other/litellm-bedrock-files-x", buckets, false),
        Err(CloudStorageError::BucketMismatch)
    );
    let prefixed = [String::from("bucket/prefix")];
    assert_eq!(
        validate_configured_file_id(
            "s3://bucket/prefix/litellm-bedrock-files-x",
            &prefixed,
            false
        ),
        Ok((
            "bucket".to_owned(),
            "prefix/litellm-bedrock-files-x".to_owned()
        ))
    );
    assert_eq!(
        validate_configured_file_id(
            "s3://bucket/prefix-evil/litellm-bedrock-files-x",
            &prefixed,
            false
        ),
        Err(CloudStorageError::Unmanaged)
    );
}

fn assert_one_pass_decoding(buckets: &[String]) {
    assert_eq!(
        validate_configured_file_id("s3://bucket/%2e%2e/x", buckets, true),
        Err(CloudStorageError::Segment)
    );
    assert_eq!(
        validate_configured_file_id("s3://bucket/litellm-bedrock-files-%252e", buckets, false),
        Ok(("bucket".to_owned(), "litellm-bedrock-files-%2e".to_owned()))
    );
}

fn foreign_bucket_rejected() {
    let configured = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text("configured".to_owned()),
    )]));
    let rejection = resolve_managed_s3_object(
        "s3://foreign/litellm-bedrock-files-x",
        &configured,
        &environment(&[]),
    )
    .expect_err("the bucket is foreign");
    let ManagedObjectError::Rejected(rejection_error) = rejection else {
        panic!("a foreign bucket is a BedrockError");
    };
    assert_bedrock_rejection(
        &rejection_error,
        "file_id bucket does not match the configured storage bucket",
    );

    let buckets = [String::from("other"), String::from("configured/prefix")];
    assert_eq!(
        validate_configured_file_id("s3://configured/plain", &buckets, false),
        Err(CloudStorageError::Unmanaged)
    );
    assert_configuration_errors_stay_original();
}

fn assert_configuration_errors_stay_original() {
    let missing = resolve_managed_s3_object(
        "s3://bucket/x",
        &params(TrustedModelCredentials::default()),
        &environment(&[]),
    )
    .expect_err("no bucket is configured");
    let ManagedObjectError::Configuration(missing_error) = missing else {
        panic!("missing configuration is not a BedrockError");
    };
    assert!(matches!(*missing_error, S3AccessError::BucketRequired));

    let malformed = params(frozen_mapping(vec![(
        "s3_bucket_name".to_owned(),
        TrustedValue::Boolean(false),
    )]));
    let invalid = resolve_managed_s3_object("s3://bucket/x", &malformed, &environment(&[]))
        .expect_err("the snapshot field is not a string");
    let ManagedObjectError::Configuration(type_error) = invalid else {
        panic!("a broken snapshot is not a BedrockError");
    };
    assert!(matches!(*type_error, S3AccessError::BucketNameType));
}

fn legacy_id_opt_in() {
    let opt_in = |value: TrustedValue| {
        frozen_mapping(vec![("allow_legacy_cloud_file_ids".to_owned(), value)]).allows_legacy()
    };
    assert!(!TrustedModelCredentials::default().allows_legacy());
    assert!(opt_in(TrustedValue::Boolean(true)));
    assert!(!opt_in(TrustedValue::Boolean(false)));
    assert!(opt_in(TrustedValue::Text("1".to_owned())));
    assert!(opt_in(TrustedValue::Text(" true ".to_owned())));
    assert!(opt_in(TrustedValue::Text("YES".to_owned())));
    assert!(opt_in(TrustedValue::Text("on".to_owned())));
    assert!(!opt_in(TrustedValue::Text("0".to_owned())));
    assert!(!opt_in(TrustedValue::Other));
    let buckets = [String::from("bucket")];
    assert_eq!(
        validate_configured_file_id("s3://bucket/legacy-object", &buckets, true),
        Ok(("bucket".to_owned(), "legacy-object".to_owned()))
    );
    assert_eq!(
        validate_configured_file_id("s3://bucket/legacy-object", &buckets, false),
        Err(CloudStorageError::Unmanaged)
    );
}

fn request_credentials_rejected() {
    let variables = environment(&[
        (BUCKET_NAME_ENV, "env-bucket"),
        (OUTPUT_BUCKET_NAME_ENV, "env-output"),
    ]);
    assert_bucket_sources(&variables);
    assert_malformed_snapshot_is_rejected(&variables);
}

fn assert_bucket_sources(variables: &BTreeMap<String, String>) {
    let trusted = S3AccessParams {
        trusted: frozen_mapping(vec![
            (
                "s3_bucket_name".to_owned(),
                TrustedValue::Text("first".to_owned()),
            ),
            (
                "s3_bucket_name".to_owned(),
                TrustedValue::Text("trusted".to_owned()),
            ),
            ("nested".to_owned(), TrustedValue::Other),
        ]),
        bucket_name: Some("request-bucket".to_owned()),
        output_bucket_name: Some("request-output".to_owned()),
    };
    assert_eq!(
        configured_bucket_name(&trusted, variables).expect("the trusted snapshot wins"),
        "trusted"
    );
    assert_eq!(
        configured_bucket_names(&trusted, variables).expect("the trusted snapshot wins"),
        ["trusted", "env-output"]
    );

    let plain = S3AccessParams {
        trusted: TrustedModelCredentials::default(),
        bucket_name: Some("request-bucket".to_owned()),
        output_bucket_name: Some("request-output".to_owned()),
    };
    assert_eq!(
        configured_bucket_name(&plain, variables).expect("request fields are ignored"),
        "env-bucket"
    );
    assert_eq!(
        configured_bucket_names(&plain, variables).expect("request fields are ignored"),
        ["env-bucket", "env-output"]
    );
}

fn assert_malformed_snapshot_is_rejected(variables: &BTreeMap<String, String>) {
    let malformed = params(frozen_mapping(vec![(
        "s3_output_bucket_name".to_owned(),
        TrustedValue::Other,
    )]));
    let error =
        configured_bucket_name(&malformed, variables).expect_err("the output field is not text");
    assert_eq!(store(&error), "output_bucket_name_type");
}

fn kms_owner_precedence() {
    let params_map = settings(&[
        ("s3_encryption_key_id", "params-key"),
        ("s3_bucket_owner", ""),
    ]);
    let optional = settings(&[
        ("s3_encryption_key_id", "request-key"),
        ("s3_bucket_owner", "request-owner"),
    ]);
    let lookup = secrets();
    let variables = environment(&[
        ("AWS_S3_ENCRYPTION_KEY_ID", "env-key"),
        ("AWS_S3_BUCKET_OWNER", "env-owner"),
    ]);
    let sources = S3SettingSources {
        params: &params_map,
        optional_params: &optional,
        secrets: &lookup,
        environment: &variables,
    };
    assert_eq!(
        resolve_s3_setting(S3Setting::EncryptionKeyId, &sources)
            .expect("the fixture lookup cannot fail")
            .map(|value| value.expose_secret().to_owned())
            .as_deref(),
        Some("params-key")
    );
    assert_eq!(
        resolve_s3_setting(S3Setting::BucketOwner, &sources)
            .expect("the fixture lookup cannot fail")
            .map(|value| value.expose_secret().to_owned())
            .as_deref(),
        Some("request-owner")
    );
}

fn s3_model_helpers() {
    assert_eq!(
        bedrock_model_id("bedrock/anthropic.claude-3-sonnet"),
        "anthropic.claude-3-sonnet"
    );
    assert_eq!(bedrock_model_id("invoke/foo"), "invoke/foo");
    assert_eq!(
        strip_with_empty_context("bedrock/anthropic.claude-3-sonnet"),
        "anthropic.claude-3-sonnet"
    );
    assert_eq!(strip_with_empty_context("invoke/foo"), "invoke/foo");
    assert_eq!(
        invoke_with_empty_context("bedrock/anthropic.claude-3-sonnet"),
        "invoke/anthropic.claude-3-sonnet"
    );
    assert_eq!(invoke_with_empty_context("invoke/foo"), "invoke/foo");
    assert_eq!(AUTHENTICATION_PARAMS.len(), 12);
    let shared = credential_cache();
    let batch = BatchFiles {
        base: AwsBase {
            credential_cache: Arc::clone(&shared),
            authentication_params: &AUTHENTICATION_PARAMS,
        },
    };
    assert!(Arc::ptr_eq(&batch.base.credential_cache, &shared));
}

fn plain_s3_uri_parser() {
    assert_eq!(
        parse_s3_uri("s3://bucket/key/path").expect("a plain URI parses"),
        ("bucket".to_owned(), "key/path".to_owned())
    );
    assert_eq!(
        parse_s3_uri("s3://bucket/").expect("an empty key parses here"),
        ("bucket".to_owned(), String::new())
    );
    assert_eq!(
        parse_s3_uri("s3:///key").expect("an empty bucket parses here"),
        (String::new(), "key".to_owned())
    );
    assert_eq!(
        parse_s3_uri("s3://bucket/a/../b").expect("traversal parses here"),
        ("bucket".to_owned(), "a/../b".to_owned())
    );
    let missing_key = parse_s3_uri("s3://bucket").expect_err("the key separator is required");
    assert_eq!(
        missing_key.to_string(),
        "Invalid S3 URI format: s3://bucket"
    );
    let wrong_scheme = parse_s3_uri("ftp://bucket/key").expect_err("the scheme is required");
    assert_eq!(
        wrong_scheme.to_string(),
        "Invalid S3 URI format: ftp://bucket/key"
    );
}

fn files_handler_initialization() {
    let shared = credential_cache();
    let handler = BedrockFilesHandler {
        base: AwsBase {
            credential_cache: Arc::clone(&shared),
            authentication_params: &AUTHENTICATION_PARAMS,
        },
        client: client(),
    };
    assert!(Arc::ptr_eq(&handler.base.credential_cache, &shared));
    assert_eq!(AUTHENTICATION_PARAMS.len(), 12);
    assert_eq!(
        decode_managed_file_id(MANAGED_FILE_ID).expect("the handler extracts the same URI"),
        MANAGED_OBJECT
    );
    let rejection =
        decode_managed_file_id("not-a-file-id").expect_err("the same rejection as the handler");
    assert_eq!(store(&rejection), "managed_file_id");
    let configured = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text("managed-bucket".to_owned()),
    )]));
    assert_eq!(
        configured_bucket_name(&configured, &environment(&[])).expect("the same bucket"),
        "managed-bucket"
    );
    let missing = configured_bucket_name(
        &params(TrustedModelCredentials::default()),
        &environment(&[]),
    )
    .expect_err("the same error as the handler");
    assert_eq!(store(&missing), "bucket_required");
}
