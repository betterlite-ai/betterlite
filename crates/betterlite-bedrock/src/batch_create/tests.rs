#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::future::ready;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObjectKind, BatchStatus};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::BatchCreateError;
use super::reply::{
    OriginalBatchRequest, create_batch_reply, decode_created_batch, openai_batch_metadata,
};
use super::request::build::{CreateBatchInput, build_create_request};
use super::request::sign::transform_create_batch_request;
use super::request::wire::{BedrockCreateRequest, S3InputDataConfig, SignedBatchRequest};
use super::tags::BedrockTag;
use super::titan::{TitanBatchUsage, titan_batch_token_usage as titan_usage};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::target::RegionInputs;

const DEFAULT_ID: &str = "00000000-0000-4000-8000-000000000000";
const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const SESSION_TOKEN: &str = "session-token-fixture";
const JOB_ARN: &str = "arn:aws:bedrock:us-west-2:1:model-invocation-job/job";

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn no_secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .expect("the fixture clock is in range")
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
}

fn fixture_credentials(access_key_id: &str, session_token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access_key_id.to_owned()),
        secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
        session_token: session_token.map(|token| SecretString::from(token.to_owned())),
    }
}

struct Sources {
    deployment: BTreeMap<String, Box<RawValue>>,
    request: BTreeMap<String, Box<RawValue>>,
    environment: BTreeMap<String, String>,
    secrets: SecretLookup,
}

impl Sources {
    fn new(deployment: &str, request: &str, environment: &[(&str, &str)]) -> Self {
        Self {
            deployment: serde_json::from_str(deployment).expect("deployment params are JSON"),
            request: serde_json::from_str(request).expect("request params are JSON"),
            environment: environment
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            secrets: no_secrets(),
        }
    }

    fn input<'text>(
        &'text self,
        model: &'text str,
        file: Option<&'text str>,
    ) -> CreateBatchInput<'text> {
        CreateBatchInput {
            model,
            input_file_id: file,
            completion_window: None,
            deployment_params: &self.deployment,
            request_params: &self.request,
            environment: &self.environment,
            secrets: &self.secrets,
            job_prefix: None,
            unique_id: DEFAULT_ID,
            region_inputs: RegionInputs::default(),
            now: now(),
        }
    }
}

fn build(input: &CreateBatchInput<'_>) -> BedrockCreateRequest {
    build_create_request(input).expect("the Bedrock request builds")
}

fn build_error(input: &CreateBatchInput<'_>) -> BatchCreateError {
    build_create_request(input).expect_err("the Bedrock request is rejected")
}

fn sign(input: CreateBatchInput<'_>) -> SignedBatchRequest {
    runtime()
        .block_on(transform_create_batch_request(
            input,
            |_settings, _region| {
                ready(Ok::<Credentials, CredentialError>(fixture_credentials(
                    ACCESS_KEY, None,
                )))
            },
        ))
        .expect("the request signs")
}

fn scripted_clock(values: &'static [i64]) -> impl Fn() -> i64 {
    let position = Cell::new(0_usize);
    move || {
        let index = position.get();
        position.set(index.saturating_add(1));
        values.get(index).copied().unwrap_or_default()
    }
}

fn empty_original() -> OriginalBatchRequest {
    OriginalBatchRequest::default()
}

fn json_of(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

#[rstest]
#[case::batch_role_required(batch_role_required)]
#[case::output_bucket_fallback(output_bucket_fallback)]
#[case::tags_validation(tags_validation)]
#[case::input_24h_window(input_24h_window)]
#[case::create_status_mapping(create_status_mapping)]
#[case::metadata_sanitization(metadata_sanitization)]
#[case::generated_job_name(generated_job_name)]
#[case::owner_and_kms(owner_and_kms)]
#[case::create_signed_request(create_signed_request)]
#[case::create_reply_defaults(create_reply_defaults)]
#[case::titan_batch_token_usage(titan_batch_token_usage)]
fn batch_create_cases(#[case] scenario: fn()) {
    scenario();
}

fn batch_role_required() {
    missing_input_file_ids_are_rejected();
    role_arn_precedence();
}

fn missing_input_file_ids_are_rejected() {
    let sources = Sources::new("{}", "{}", &[]);
    assert!(matches!(
        build_error(&sources.input("m", None)),
        BatchCreateError::InputFileIdRequired
    ));
    assert!(matches!(
        build_error(&sources.input("m", Some(""))),
        BatchCreateError::InputFileIdRequired
    ));
    assert!(matches!(
        build_error(&sources.input("m", Some("not-s3"))),
        BatchCreateError::S3(_)
    ));
    assert!(matches!(
        build_error(&sources.input("m", Some("s3://bucket/key"))),
        BatchCreateError::RoleArnRequired
    ));
}

fn role_arn_precedence() {
    let optional_role = Sources::new(
        "{}",
        r#"{"aws_batch_role_arn": "arn:aws:iam::2:role/request"}"#,
        &[("AWS_BATCH_ROLE_ARN", "arn:aws:iam::3:role/environment")],
    );
    assert_eq!(
        build(&optional_role.input("m", Some("s3://bucket/key"))).role_arn,
        "arn:aws:iam::2:role/request"
    );

    let empty_deployment = Sources::new(
        r#"{"aws_batch_role_arn": ""}"#,
        "{}",
        &[("AWS_BATCH_ROLE_ARN", "arn:aws:iam::3:role/environment")],
    );
    assert_eq!(
        build(&empty_deployment.input("m", Some("s3://bucket/key"))).role_arn,
        "arn:aws:iam::3:role/environment"
    );

    let empty_environment = Sources::new(
        r#"{"aws_batch_role_arn": ""}"#,
        r#"{"aws_batch_role_arn": ""}"#,
        &[("AWS_BATCH_ROLE_ARN", "")],
    );
    assert!(matches!(
        build_error(&empty_environment.input("m", Some("s3://bucket/key"))),
        BatchCreateError::RoleArnRequired
    ));

    let role_only = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/batch"}"#,
        "{}",
        &[],
    );
    assert!(matches!(
        build_error(&role_only.input("", Some("s3://bucket/key"))),
        BatchCreateError::ModelRequired
    ));
}

fn output_bucket_fallback() {
    output_bucket_precedence();
    output_bucket_defaults();
}

fn output_bucket_precedence() {
    let configured = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_output_bucket_name": "out-bucket"}"#,
        "{}",
        &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-bucket")],
    );
    assert_eq!(
        build(&configured.input("m", Some("s3://in-bucket/in.jsonl")))
            .output_data_config
            .s3_output_data_config
            .s3_uri,
        "s3://out-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );

    let empty_deployment = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_output_bucket_name": ""}"#,
        "{}",
        &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-bucket")],
    );
    assert_eq!(
        build(&empty_deployment.input("m", Some("s3://in-bucket/in.jsonl")))
            .output_data_config
            .s3_output_data_config
            .s3_uri,
        "s3://env-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );

    let request_only = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        r#"{"s3_output_bucket_name": "request-bucket"}"#,
        &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-bucket")],
    );
    assert_eq!(
        build(&request_only.input("m", Some("s3://in-bucket/in.jsonl")))
            .output_data_config
            .s3_output_data_config
            .s3_uri,
        "s3://env-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );
}

fn output_bucket_defaults() {
    let from_environment = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[("AWS_S3_OUTPUT_BUCKET_NAME", "env-bucket")],
    );
    assert_eq!(
        build(&from_environment.input("m", Some("s3://in-bucket/in.jsonl")))
            .output_data_config
            .s3_output_data_config
            .s3_uri,
        "s3://env-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );

    let empty_environment = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[("AWS_S3_OUTPUT_BUCKET_NAME", "")],
    );
    assert_eq!(
        build(&empty_environment.input("m", Some("s3://in-bucket/in.jsonl")))
            .output_data_config
            .s3_output_data_config
            .s3_uri,
        "s3://in-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );

    let input_bucket = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[],
    );
    let request = build(&input_bucket.input("m", Some("s3://in-bucket/in.jsonl")));
    assert_eq!(
        request.input_data_config.s3_input_data_config.s3_uri,
        "s3://in-bucket/in.jsonl"
    );
    assert_eq!(
        request.output_data_config.s3_output_data_config.s3_uri,
        "s3://in-bucket/litellm-batch-outputs/litellm-batch-00000000/"
    );
}

fn tags_validation() {
    tag_precedence();
    tags_fall_back_and_override();
    invalid_tags_are_rejected();
}

fn tag_precedence() {
    let optional = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        r#"{"bedrock_tags": [{"key": "env", "value": "prod"}]}"#,
        &[],
    );
    assert_eq!(
        build(&optional.input("m", Some("s3://b/k"))).tags,
        Some(vec![BedrockTag {
            key: "env".to_owned(),
            value: "prod".to_owned(),
        }])
    );

    let both = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "bedrock_tags": [{"key": "team", "value": "genai", "extra": "dropped"}]}"#,
        r#"{"bedrock_tags": [{"key": "env", "value": "prod"}]}"#,
        &[],
    );
    assert_eq!(
        build(&both.input("m", Some("s3://b/k"))).tags,
        Some(vec![BedrockTag {
            key: "team".to_owned(),
            value: "genai".to_owned(),
        }])
    );
}

fn tags_fall_back_and_override() {
    let empty_override = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "bedrock_tags": []}"#,
        r#"{"bedrock_tags": [{"key": "env", "value": "prod"}]}"#,
        &[],
    );
    assert_eq!(
        build(&empty_override.input("m", Some("s3://b/k"))).tags,
        Some(Vec::new())
    );

    let null_falls_through = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "bedrock_tags": null}"#,
        r#"{"bedrock_tags": [{"key": "env", "value": "prod"}]}"#,
        &[],
    );
    assert_eq!(
        build(&null_falls_through.input("m", Some("s3://b/k")))
            .tags
            .map(|tags| tags.len()),
        Some(1)
    );

    let request_null = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        r#"{"bedrock_tags": null}"#,
        &[],
    );
    assert_eq!(build(&request_null.input("m", Some("s3://b/k"))).tags, None);
}

fn invalid_tags_are_rejected() {
    for invalid in [
        r#"["application=genai"]"#,
        r#"[{"key": "application"}]"#,
        r#"[{"value": "genai"}]"#,
        r#"[{"key": "application", "value": 42}]"#,
        r#"{"key": "application", "value": "genai"}"#,
        r#""application=genai""#,
    ] {
        let sources = Sources::new(
            r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
            &format!(r#"{{"bedrock_tags": {invalid}}}"#),
            &[],
        );
        let error = build_error(&sources.input("m", Some("s3://b/k")));
        assert!(
            matches!(error, BatchCreateError::InvalidTags { .. }),
            "{invalid} was accepted"
        );
    }
}

fn input_24h_window() {
    let sources = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[],
    );
    let mut input = sources.input("m", Some("s3://b/k"));
    assert_eq!(build(&input).timeout_duration_in_hours, None);
    input.completion_window = Some("");
    assert_eq!(build(&input).timeout_duration_in_hours, None);
    input.completion_window = Some("48h");
    assert_eq!(build(&input).timeout_duration_in_hours, None);
    input.completion_window = Some("24H");
    assert_eq!(build(&input).timeout_duration_in_hours, None);
    input.completion_window = Some("24h");
    assert_eq!(build(&input).timeout_duration_in_hours, Some(24));
}

fn create_status_mapping() {
    status_table();
    status_defaults();
}

fn status_table() {
    let cases = [
        ("Submitted", false, BatchStatus::Validating),
        ("Validating", false, BatchStatus::Validating),
        ("Scheduled", false, BatchStatus::InProgress),
        ("InProgress", true, BatchStatus::InProgress),
        ("PartiallyCompleted", false, BatchStatus::Completed),
        ("Completed", false, BatchStatus::Completed),
        ("Failed", false, BatchStatus::Failed),
        ("Stopping", false, BatchStatus::Cancelling),
        ("Stopped", false, BatchStatus::Cancelled),
        ("Expired", false, BatchStatus::Expired),
        ("SomeFutureStatus", false, BatchStatus::Validating),
    ];
    for (bedrock, in_progress, expected) in cases {
        let body = format!(r#"{{"jobArn": "{JOB_ARN}", "status": "{bedrock}"}}"#);
        let created = decode_created_batch(body.as_bytes()).expect("the body decodes");
        assert_eq!(created.in_progress, in_progress, "{bedrock}");
        assert_eq!(created.status, expected, "{bedrock}");
    }
}

fn status_defaults() {
    let missing = decode_created_batch(format!(r#"{{"jobArn": "{JOB_ARN}"}}"#).as_bytes())
        .expect("a missing status decodes");
    assert_eq!(missing.status, BatchStatus::Validating);
    let null_status =
        decode_created_batch(format!(r#"{{"jobArn": "{JOB_ARN}", "status": null}}"#).as_bytes())
            .expect("a null status decodes");
    assert_eq!(null_status.status, BatchStatus::Validating);
    let numeric_status =
        decode_created_batch(format!(r#"{{"jobArn": "{JOB_ARN}", "status": 7}}"#).as_bytes())
            .expect("a numeric status decodes");
    assert_eq!(numeric_status.status, BatchStatus::Validating);
    assert!(matches!(
        decode_created_batch(b"not-json"),
        Err(BatchCreateError::ResponseParse { .. })
    ));
    assert!(matches!(
        decode_created_batch(br#"{"jobArn": 7, "status": "Submitted"}"#),
        Err(BatchCreateError::ResponseParse { .. })
    ));
    let array = decode_created_batch(b"[]").expect_err("an array is not a batch response");
    assert!(matches!(array, BatchCreateError::ResponseParse { .. }));
    assert!(
        array
            .to_string()
            .contains("the response body must be a JSON object")
    );
    assert!(matches!(
        decode_created_batch(br#"{"jobArn": "first", "jobArn": "second"}"#),
        Err(BatchCreateError::ResponseParse { .. })
    ));
    let padded = decode_created_batch(b" \n {\"jobArn\": \"job\"}")
        .expect("whitespace before the object is allowed");
    assert_eq!(padded.job_arn, "job");
}

fn metadata_sanitization() {
    metadata_values();
    metadata_shapes();
}

fn metadata_values() {
    let metadata = json_of(
        r#"{"user": "alice", "none": null, "number": 1, "float": 0.5, "flag": true, "standard_logging_guardrail_information": {"x": 1}, "payload": {"a": 1}, "list": [1, "two"]}"#,
    );
    let sanitized = openai_batch_metadata(&metadata);
    assert_eq!(sanitized.get("user").map(String::as_str), Some("alice"));
    assert_eq!(sanitized.get("none"), None);
    assert_eq!(
        sanitized.get("standard_logging_guardrail_information"),
        None
    );
    assert_eq!(sanitized.get("number").map(String::as_str), Some("1"));
    assert_eq!(sanitized.get("float").map(String::as_str), Some("0.5"));
    assert_eq!(sanitized.get("flag").map(String::as_str), Some("true"));
    assert_eq!(
        sanitized.get("payload").map(String::as_str),
        Some(r#"{"a":1}"#)
    );
    assert_eq!(
        sanitized.get("list").map(String::as_str),
        Some(r#"[1,"two"]"#)
    );
}

fn metadata_shapes() {
    assert!(openai_batch_metadata(&json_of("[1, 2]")).is_empty());
    assert!(openai_batch_metadata(&json_of(r#""text""#)).is_empty());
    assert!(openai_batch_metadata(&json_of("3")).is_empty());
    assert!(openai_batch_metadata(&Json::Null).is_empty());
    let nul = json_of(r#"{"key\u0000name": "a\u0000b", "nested": {"inner": "c\u0000d"}}"#);
    let sanitized = openai_batch_metadata(&nul);
    assert_eq!(
        sanitized.get("key\u{0}name").map(String::as_str),
        Some("a\u{0}b")
    );
    assert_eq!(
        sanitized.get("nested").map(String::as_str),
        Some(r#"{"inner":"cd"}"#)
    );
}

fn generated_job_name() {
    let sources = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[],
    );
    let request = build(&sources.input("bedrock/anthropic.claude", Some("s3://b/k")));
    assert_eq!(request.job_name, "litellm-batch-00000000");
    assert_eq!(request.model_id, "bedrock/anthropic.claude");

    let mut input = sources.input("m", Some("s3://b/k"));
    input.job_prefix = Some("custom");
    input.unique_id = "1234567890abcdef";
    assert_eq!(build(&input).job_name, "custom-batch-12345678");

    input.unique_id = "abc";
    assert_eq!(build(&input).job_name, "custom-batch-abc");
}

fn owner_and_kms() {
    owner_precedence();
    kms_is_output_only();
    builder_omissions();
}

fn owner_precedence() {
    let configured = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_bucket_owner": "111111111111", "s3_encryption_key_id": "kms-key"}"#,
        "{}",
        &[],
    );
    let deployment_request = build(&configured.input("m", Some("s3://b/k")));
    assert_eq!(
        deployment_request
            .input_data_config
            .s3_input_data_config
            .s3_bucket_owner
            .as_deref(),
        Some("111111111111")
    );

    let optional = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        r#"{"s3_bucket_owner": "222222222222", "s3_encryption_key_id": "optional-key"}"#,
        &[],
    );
    let optional_request = build(&optional.input("m", Some("s3://b/k")));
    assert_eq!(
        optional_request
            .input_data_config
            .s3_input_data_config
            .s3_bucket_owner
            .as_deref(),
        Some("222222222222")
    );
    empty_owner_falls_back();
}

fn empty_owner_falls_back() {
    let empty = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_bucket_owner": "", "s3_encryption_key_id": ""}"#,
        "{}",
        &[
            ("AWS_S3_BUCKET_OWNER", "444444444444"),
            ("AWS_S3_ENCRYPTION_KEY_ID", "fallback-key"),
        ],
    );
    let empty_request = build(&empty.input("m", Some("s3://b/k")));
    assert_eq!(
        empty_request
            .input_data_config
            .s3_input_data_config
            .s3_bucket_owner
            .as_deref(),
        Some("444444444444")
    );
}

fn kms_is_output_only() {
    let environment = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        "{}",
        &[
            ("AWS_S3_BUCKET_OWNER", "333333333333"),
            ("AWS_S3_ENCRYPTION_KEY_ID", "env-key"),
        ],
    );
    let request = build(&environment.input("m", Some("s3://b/k")));
    assert_eq!(
        request
            .output_data_config
            .s3_output_data_config
            .s3_bucket_owner
            .as_deref(),
        Some("333333333333")
    );
    assert_eq!(
        request
            .output_data_config
            .s3_output_data_config
            .s3_encryption_key_id
            .as_deref(),
        Some("env-key")
    );
    let serialized =
        serde_json::to_string(&request.input_data_config).expect("the input serializes");
    assert!(!serialized.contains("s3EncryptionKeyId"));
}

fn builder_omissions() {
    let unset = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_bucket_owner": ""}"#,
        "{}",
        &[],
    );
    let request = build(&unset.input("m", Some("s3://b/k")));
    assert_eq!(
        request
            .input_data_config
            .s3_input_data_config
            .s3_bucket_owner,
        None
    );
    assert_eq!(
        request
            .output_data_config
            .s3_output_data_config
            .s3_bucket_owner,
        None
    );
    let serialized =
        serde_json::to_string(&request.output_data_config).expect("the output serializes");
    assert!(!serialized.contains("s3BucketOwner"));
    assert!(!serialized.contains("s3EncryptionKeyId"));

    let empty_owner = S3InputDataConfig {
        s3_uri: "s3://b/k".to_owned(),
        s3_bucket_owner: Some(String::new()),
    };
    assert!(
        serde_json::to_string(&empty_owner)
            .expect("the input serializes")
            .contains(r#""s3BucketOwner":"""#)
    );
}

fn create_signed_request() {
    signed_envelope();
    signed_partition_dns();
}

fn signed_envelope() {
    let sources = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "s3_output_bucket_name": "out-bucket", "aws_region_name": "us-west-2", "aws_access_key_id": "AKIDEXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "bedrock_tags": [{"key": "team", "value": "genai"}]}"#,
        "{}",
        &[],
    );
    let mut input = sources.input("m", Some("s3://in-bucket/in.jsonl"));
    input.completion_window = Some("24h");
    let signed = sign(input);
    assert_eq!(signed.method, "POST");
    assert_eq!(
        signed.url,
        "https://bedrock.us-west-2.amazonaws.com/model-invocation-job"
    );
    assert_eq!(
        signed.data,
        r#"{"modelId": "m", "jobName": "litellm-batch-00000000", "inputDataConfig": {"s3InputDataConfig": {"s3Uri": "s3://in-bucket/in.jsonl"}}, "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://out-bucket/litellm-batch-outputs/litellm-batch-00000000/"}}, "roleArn": "arn:aws:iam::1:role/r", "tags": [{"key": "team", "value": "genai"}], "timeoutDurationInHours": 24}"#
    );
    assert!(!signed.data.contains("clientRequestToken"));
    assert_eq!(
        signed.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        signed.headers.get("X-Amz-Date").map(String::as_str),
        Some("20260101T000000Z")
    );
    assert!(
        signed
            .headers
            .get("Authorization")
            .is_some_and(|value| value.contains(ACCESS_KEY))
    );
}

fn signed_partition_dns() {
    let china = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "aws_region_name": "cn-north-1"}"#,
        "{}",
        &[],
    );
    assert_eq!(
        sign(china.input("m", Some("s3://b/k"))).url,
        "https://bedrock.cn-north-1.amazonaws.com.cn/model-invocation-job"
    );
}

fn create_reply_defaults() {
    reply_defaults();
    reply_supplied_request();
    reply_in_progress_timestamps();
}

fn reply_defaults() {
    let clock = scripted_clock(&[111, 222]);
    let reply = create_batch_reply(
        format!(r#"{{"jobArn": "{JOB_ARN}", "status": "Submitted"}}"#).as_bytes(),
        empty_original(),
        &clock,
    )
    .expect("the reply projects");
    assert_eq!(reply.id, JOB_ARN);
    assert_eq!(reply.object, BatchObjectKind::Batch);
    assert_eq!(reply.endpoint, "/v1/chat/completions");
    assert_eq!(reply.input_file_id, "");
    assert_eq!(reply.completion_window, "24h");
    assert_eq!(reply.created_at, 111);
    assert_eq!(reply.in_progress_at, None);
    assert!(reply.metadata.is_empty());
    assert_unset_fields(&reply);
    let parsed: OriginalBatchRequest = serde_json::from_str("{}").expect("an empty request parses");
    assert_eq!(parsed, empty_original());
    assert_eq!(parsed.metadata, json_of("{}"));
}

fn assert_unset_fields(reply: &super::reply::BatchCreateReply) {
    assert!(reply.errors.is_none());
    assert!(reply.output_file_id.is_none());
    assert!(reply.error_file_id.is_none());
    assert!(reply.expires_at.is_none());
    assert!(reply.finalizing_at.is_none());
    assert!(reply.completed_at.is_none());
    assert!(reply.failed_at.is_none());
    assert!(reply.expired_at.is_none());
    assert!(reply.cancelling_at.is_none());
    assert!(reply.cancelled_at.is_none());
    assert!(reply.request_counts.is_none());
    assert!(reply.model.is_none());
    assert!(reply.usage.is_none());
}

fn reply_supplied_request() {
    let original = OriginalBatchRequest {
        endpoint: "/v1/embeddings".to_owned(),
        input_file_id: "s3://b/in.jsonl".to_owned(),
        completion_window: "48h".to_owned(),
        metadata: json_of(
            r#"{"user": "alice", "none": null, "standard_logging_guardrail_information": {"x": 1}}"#,
        ),
    };
    let reply = create_batch_reply(
        br#"{"jobArn": "job", "status": "Completed"}"#,
        original,
        &scripted_clock(&[111]),
    )
    .expect("the reply projects");
    assert_eq!(reply.endpoint, "/v1/embeddings");
    assert_eq!(reply.input_file_id, "s3://b/in.jsonl");
    assert_eq!(reply.completion_window, "48h");
    assert_eq!(
        reply.metadata.get("user").map(String::as_str),
        Some("alice")
    );
    assert!(!reply.metadata.contains_key("none"));
    assert!(
        !reply
            .metadata
            .contains_key("standard_logging_guardrail_information")
    );
}

fn reply_in_progress_timestamps() {
    let reply = create_batch_reply(
        br#"{"jobArn": "job", "status": "InProgress"}"#,
        empty_original(),
        &scripted_clock(&[111, 222]),
    )
    .expect("the reply projects");
    assert_eq!(reply.created_at, 111);
    assert_eq!(reply.in_progress_at, Some(222));

    let scheduled = create_batch_reply(
        br#"{"jobArn": "job", "status": "Scheduled"}"#,
        empty_original(),
        &scripted_clock(&[111, 222]),
    )
    .expect("the reply projects");
    assert_eq!(scheduled.in_progress_at, None);

    let supplied = OriginalBatchRequest {
        endpoint: String::new(),
        input_file_id: String::new(),
        completion_window: String::new(),
        metadata: Json::Null,
    };
    let empty = create_batch_reply(br#"{"jobArn": ""}"#, supplied, &scripted_clock(&[5]))
        .expect("the reply projects");
    assert_eq!(empty.endpoint, "");
    assert_eq!(empty.input_file_id, "");
    assert_eq!(empty.completion_window, "");
    assert_eq!(empty.created_at, 5);
    assert!(empty.metadata.is_empty());
}

fn titan_batch_token_usage() {
    titan_usage_values();
    titan_usage_rejections();
}

fn titan_usage_values() {
    let usage = |text: &str| titan_usage(&json_of(text));
    assert_eq!(
        usage(r#"{"embedding": [1], "inputTextTokenCount": 7}"#),
        Some(TitanBatchUsage {
            prompt_tokens: 7,
            completion_tokens: 0,
            total_tokens: 7,
        })
    );
    assert_eq!(
        usage(r#"{"embeddingsByType": null, "inputTextTokenCount": 0}"#),
        Some(TitanBatchUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        })
    );
    assert_eq!(
        usage(r#"{"embedding": null, "inputTextTokenCount": -3}"#),
        Some(TitanBatchUsage {
            prompt_tokens: -3,
            completion_tokens: 0,
            total_tokens: -3,
        })
    );
}

fn titan_usage_rejections() {
    let usage = |text: &str| titan_usage(&json_of(text));
    assert_eq!(usage(r#"{"inputTextTokenCount": 7}"#), None);
    assert_eq!(usage(r#"{"embedding": [1]}"#), None);
    assert_eq!(
        usage(r#"{"embedding": [1], "inputTextTokenCount": true}"#),
        None
    );
    assert_eq!(
        usage(r#"{"embedding": [1], "inputTextTokenCount": 1.5}"#),
        None
    );
    assert_eq!(
        usage(r#"{"embedding": [1], "inputTextTokenCount": "7"}"#),
        None
    );
    assert_eq!(
        usage(r#"{"embedding": [1], "inputTextTokenCount": null}"#),
        None
    );
    assert_eq!(usage("[1, 2]"), None);
}

#[test]
fn replays_the_bedrock_services_batch_create_corpus() {
    let seeds = replay_batch_create_corpus();
    assert!(seeds >= 4, "the corpus holds only {seeds} seeds");
}

fn replay_batch_create_corpus() -> usize {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_batch_create");
    let entries = fs::read_dir(corpus).expect("the corpus directory exists");
    let mut seeds = 0_usize;
    for entry in entries {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let expected = name.starts_with("valid") || name.starts_with("boundary-valid");
        assert_eq!(
            decode_created_batch(&bytes).is_ok(),
            expected,
            "seed {name}"
        );
        seeds = seeds.saturating_add(1);
    }
    seeds
}

mod invariants;
