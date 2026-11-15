use http::Method;
use rstest::rstest;
use secrecy::SecretString;

use crate::finops::focus::destinations::focus_s3_destination::{
    CONTENT_TYPE, FocusS3Destination, S3DeliveryError, S3Settings, S3SettingsError,
};
use crate::finops::focus::destinations::object_key;
use crate::finops::focus::destinations::tests::support::{
    BUCKET, CONTENT, FILENAME, FakeS3Http, PREFIX, START_EPOCH, header, s3_bucket_only_settings,
    s3_connection, s3_destination, s3_destination_with, s3_settings, window,
};
use crate::focus_records::destinations::FocusDestination as _;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::CredentialError;

const CONTENT_SHA256: &str = "15c4f9b8f53c8259dc69286ee88545cfd56c06623f82df6b73dd65717978daf8";
const ENVIRONMENT_CREDENTIALS: [(&str, &str); 3] = [
    ("AWS_ACCESS_KEY_ID", "env-access"),
    ("AWS_SECRET_ACCESS_KEY", "env-secret"),
    ("AWS_SESSION_TOKEN", "env-token"),
];

fn authorization(http: &FakeS3Http) -> String {
    header(&http.sent()[0], "authorization").unwrap().to_owned()
}

#[test]
fn init_requires_a_bucket_name() {
    let connection = s3_connection(&[], &FakeS3Http::answering(200, b""));
    let Err(error) = FocusS3Destination::new(connection, PREFIX, S3Settings::default()) else {
        panic!("a destination without a bucket was built");
    };
    assert!(matches!(error, S3SettingsError::MissingBucketName));
}

#[test]
fn init_rejects_an_empty_bucket_name() {
    let settings = S3Settings {
        bucket_name: Some(String::new()),
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
    };
    let connection = s3_connection(&[], &FakeS3Http::answering(200, b""));
    let Err(error) = FocusS3Destination::new(connection, PREFIX, settings) else {
        panic!("a destination with an empty bucket was built");
    };
    assert!(matches!(error, S3SettingsError::MissingBucketName));
}

#[rstest]
#[case::trailing("exports///", "exports")]
#[case::only_slashes("///", "")]
#[case::leading_kept("/exports/", "/exports")]
#[test]
fn init_trims_only_trailing_slashes_from_the_prefix(#[case] prefix: &str, #[case] expected: &str) {
    let (destination, _http) = s3_destination(prefix, s3_bucket_only_settings());
    assert_eq!(destination.prefix, expected);
}

#[test]
fn init_accepts_every_optional_client_option() {
    let (destination, _http) = s3_destination(PREFIX, s3_settings());
    assert_eq!(destination.bucket_name, BUCKET);
    assert_eq!(
        destination.client_options.region_name.as_deref(),
        Some("us-east-1")
    );
    assert!(destination.client_options.aws_session_token.is_some());
}

#[tokio::test]
async fn deliver_uploads_the_export_with_a_put() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let sent = http.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::PUT);
    assert_eq!(sent[0].body, CONTENT);
}

#[tokio::test]
async fn deliver_awaits_the_async_upload() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    let time_window = window(START_EPOCH, "daily");
    let delivery = destination.deliver(CONTENT, &time_window, FILENAME);
    assert!(http.sent().is_empty());
    delivery.await.unwrap();
    assert_eq!(http.sent().len(), 1);
}

#[tokio::test]
async fn deliver_builds_the_daily_object_key() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        http.sent()[0].url,
        format!("https://s3.example.test/{BUCKET}/{PREFIX}/date%3D2026-01-02/{FILENAME}")
    );
}

#[tokio::test]
async fn deliver_builds_the_hourly_object_key() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "hourly"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        http.sent()[0].url,
        format!("https://s3.example.test/{BUCKET}/{PREFIX}/date%3D2026-01-02/hour%3D03/{FILENAME}")
    );
}

#[tokio::test]
async fn deliver_reports_transport_failures() {
    let http = FakeS3Http::failing();
    let destination = s3_destination_with(PREFIX, s3_settings(), &[], &http);
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        S3DeliveryError::Transport(S3TransportError::MissingBucket)
    ));
}

#[tokio::test]
async fn deliver_uploads_empty_content() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(&[], &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(http.sent()[0].body.is_empty());
}

#[rstest]
#[case::ok(200)]
#[case::no_content(204)]
#[case::last_success(299)]
#[tokio::test]
async fn deliver_accepts_success_statuses(#[case] status: u16) {
    let http = FakeS3Http::answering(status, b"");
    let destination = s3_destination_with(PREFIX, s3_settings(), &[], &http);
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
}

#[rstest]
#[case::multiple_choices(
    300,
    b"<Error><Code>AccessDenied</Code></Error>",
    "<Error><Code>AccessDenied</Code></Error>"
)]
#[case::forbidden(403, b"denied", "denied")]
#[case::server_error(500, b"", "")]
#[case::non_utf8_body(500, &[0xff, 0xfe], "")]
#[tokio::test]
async fn deliver_rejects_other_statuses(
    #[case] status: u16,
    #[case] response: &[u8],
    #[case] expected_body: &str,
) {
    let http = FakeS3Http::answering(status, response);
    let destination = s3_destination_with(PREFIX, s3_settings(), &[], &http);
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let S3DeliveryError::Status { status: seen, body } = error else {
        panic!("a status {status} response was not reported");
    };
    assert_eq!(seen, status);
    assert_eq!(body, expected_body);
}

#[rstest]
#[case::daily(PREFIX, START_EPOCH, "daily", "exports/date=2026-01-02")]
#[case::hourly_midnight(PREFIX, 1_767_312_000, "hourly", "exports/date=2026-01-02/hour=00")]
#[case::hourly_morning(PREFIX, START_EPOCH, "hourly", "exports/date=2026-01-02/hour=03")]
#[case::hourly_late(PREFIX, 1_767_394_800, "hourly", "exports/date=2026-01-02/hour=23")]
#[case::interval(PREFIX, START_EPOCH, "interval", "exports/date=2026-01-02")]
#[case::all(PREFIX, START_EPOCH, "all", "exports/date=2026-01-02")]
#[case::uppercase(PREFIX, START_EPOCH, "HOURLY", "exports/date=2026-01-02")]
#[case::empty_prefix("", START_EPOCH, "daily", "date=2026-01-02")]
#[case::trailing_slashes("exports///", START_EPOCH, "hourly", "exports/date=2026-01-02/hour=03")]
#[case::leading_slash("/exports/", START_EPOCH, "daily", "/exports/date=2026-01-02")]
#[test]
fn object_key_partitions_by_date_and_hour(
    #[case] prefix: &str,
    #[case] epoch: i64,
    #[case] frequency: &str,
    #[case] directory: &str,
) {
    let (destination, _http) = s3_destination(prefix, s3_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(epoch, frequency), FILENAME);
    assert_eq!(key, format!("{directory}/{FILENAME}"));
}

#[test]
fn kwargs_omits_absent_client_options() {
    let (destination, _http) = s3_destination(PREFIX, s3_bucket_only_settings());
    let options = &destination.client_options;
    assert!(options.region_name.is_none());
    assert!(options.endpoint_url.is_none());
    assert!(options.aws_access_key_id.is_none());
    assert!(options.aws_secret_access_key.is_none());
    assert!(options.aws_session_token.is_none());
}

#[test]
fn kwargs_omits_empty_client_options() {
    let settings = S3Settings {
        bucket_name: Some(BUCKET.to_owned()),
        region_name: Some(String::new()),
        endpoint_url: Some(String::new()),
        aws_access_key_id: Some(SecretString::from("")),
        aws_secret_access_key: Some(SecretString::from("")),
        aws_session_token: Some(SecretString::from("")),
    };
    let (destination, _http) = s3_destination(PREFIX, settings);
    let options = &destination.client_options;
    assert!(options.region_name.is_none());
    assert!(options.endpoint_url.is_none());
    assert!(options.aws_access_key_id.is_none());
    assert!(options.aws_secret_access_key.is_none());
    assert!(options.aws_session_token.is_none());
}

#[tokio::test]
async fn kwargs_reach_the_upload() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let sent = http.sent();
    let request = &sent[0];
    assert!(request.url.starts_with("https://s3.example.test/"));
    assert!(authorization(&http).contains("Credential=access-key/20260102/us-east-1/s3/"));
    assert_eq!(
        header(request, "x-amz-security-token"),
        Some("session-token")
    );
}

#[tokio::test]
async fn kwargs_trim_a_trailing_slash_from_the_endpoint() {
    let settings = S3Settings {
        endpoint_url: Some("https://s3.example.test/".to_owned()),
        ..s3_settings()
    };
    let (destination, http) = s3_destination(PREFIX, settings);
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(
        http.sent()[0]
            .url
            .starts_with(&format!("https://s3.example.test/{BUCKET}/"))
    );
}

#[tokio::test]
async fn kwargs_fall_back_to_the_environment_credentials() {
    let http = FakeS3Http::answering(200, b"");
    let destination = s3_destination_with(
        PREFIX,
        s3_bucket_only_settings(),
        &ENVIRONMENT_CREDENTIALS,
        &http,
    );
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(authorization(&http).contains("Credential=env-access/20260102/"));
    assert_eq!(
        header(&http.sent()[0], "x-amz-security-token"),
        Some("env-token")
    );
}

#[tokio::test]
async fn kwargs_empty_credentials_fall_back_to_the_environment() {
    let settings = S3Settings {
        bucket_name: Some(BUCKET.to_owned()),
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: Some(SecretString::from("")),
        aws_secret_access_key: Some(SecretString::from("")),
        aws_session_token: Some(SecretString::from("")),
    };
    let http = FakeS3Http::answering(200, b"");
    let destination = s3_destination_with(PREFIX, settings, &ENVIRONMENT_CREDENTIALS, &http);
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(authorization(&http).contains("Credential=env-access/20260102/"));
}

#[tokio::test]
async fn kwargs_without_any_credentials_fail_before_sending() {
    let (destination, http) = s3_destination(PREFIX, s3_bucket_only_settings());
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        S3DeliveryError::Transport(S3TransportError::Credentials(CredentialError::Unavailable))
    ));
    assert!(http.sent().is_empty());
}

#[tokio::test]
async fn kwargs_with_only_an_access_key_fail_before_sending() {
    let settings = S3Settings {
        bucket_name: Some(BUCKET.to_owned()),
        region_name: None,
        endpoint_url: None,
        aws_access_key_id: Some(SecretString::from("access-key")),
        aws_secret_access_key: None,
        aws_session_token: None,
    };
    let http = FakeS3Http::answering(200, b"");
    let destination = s3_destination_with(PREFIX, settings, &ENVIRONMENT_CREDENTIALS, &http);
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        S3DeliveryError::Transport(S3TransportError::Credentials(CredentialError::Unavailable))
    ));
    assert!(http.sent().is_empty());
}

#[rstest]
#[case::configured(Some("eu-west-1"), &[("AWS_DEFAULT_REGION", "ap-south-1")], "eu-west-1")]
#[case::default_variable(None, &[("AWS_DEFAULT_REGION", "ap-south-1"), ("AWS_REGION", "eu-central-1")], "ap-south-1")]
#[case::empty_default_variable(None, &[("AWS_DEFAULT_REGION", ""), ("AWS_REGION", "eu-central-1")], "us-east-1")]
#[case::empty_configured(Some(""), &[("AWS_DEFAULT_REGION", "ap-south-1")], "ap-south-1")]
#[case::region_variables_ignored(None, &[("AWS_REGION", "eu-central-1"), ("AWS_REGION_NAME", "ap-south-1")], "us-east-1")]
#[case::unset_region(None, &[], "us-east-1")]
#[tokio::test]
async fn kwargs_resolve_the_signing_region(
    #[case] region: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = S3Settings {
        region_name: region.map(str::to_owned),
        ..s3_bucket_only_settings()
    };
    let mut variables = ENVIRONMENT_CREDENTIALS.to_vec();
    variables.extend_from_slice(environment);
    let http = FakeS3Http::answering(200, b"");
    let destination = s3_destination_with(PREFIX, settings, &variables, &http);
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(authorization(&http).contains(&format!("/{expected}/s3/aws4_request")));
}

#[tokio::test]
async fn kwargs_reject_an_invalid_region_before_sending() {
    let settings = S3Settings {
        region_name: Some("US East".to_owned()),
        ..s3_settings()
    };
    let (destination, http) = s3_destination(PREFIX, settings);
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        S3DeliveryError::Transport(S3TransportError::Region { .. })
    ));
    assert!(http.sent().is_empty());
}

#[tokio::test]
async fn upload_uses_the_octet_stream_content_type() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(CONTENT_TYPE, "application/octet-stream");
    assert_eq!(
        header(&http.sent()[0], "content-type"),
        Some("application/octet-stream")
    );
}

#[tokio::test]
async fn upload_signs_the_put_with_the_payload_hash() {
    let (destination, http) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let sent = http.sent();
    let request = &sent[0];
    assert_eq!(
        header(request, "x-amz-content-sha256"),
        Some(CONTENT_SHA256)
    );
    assert_eq!(header(request, "x-amz-date"), Some("20260102T030405Z"));
    let signature = authorization(&http);
    assert!(
        signature.starts_with(
            "AWS4-HMAC-SHA256 Credential=access-key/20260102/us-east-1/s3/aws4_request"
        )
    );
    assert!(signature.contains("content-type"));
    assert!(signature.contains("x-amz-content-sha256"));
}

#[tokio::test]
async fn upload_sends_to_the_aws_host_without_an_endpoint() {
    let settings = S3Settings {
        endpoint_url: None,
        ..s3_settings()
    };
    let (destination, http) = s3_destination(PREFIX, settings);
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        http.sent()[0].url,
        format!(
            "https://{BUCKET}.s3.us-east-1.amazonaws.com/{PREFIX}/date%3D2026-01-02/{FILENAME}"
        )
    );
}
