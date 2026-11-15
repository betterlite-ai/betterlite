use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::focus_s3_destination::{
    CONTENT_TYPE, FocusS3Destination, S3DeliveryError, S3Settings, S3SettingsError,
};
use crate::finops::focus::destinations::object_key;
use crate::finops::focus::destinations::tests::support::{
    BUCKET, CONTENT, FILENAME, FakeS3Client, NetworkDown, PREFIX, START_EPOCH,
    s3_bucket_only_settings, s3_destination, s3_settings, window,
};
use crate::focus_records::destinations::FocusDestination as _;

#[test]
fn init_requires_a_bucket_name() {
    let Err(error) =
        FocusS3Destination::new(FakeS3Client::default(), PREFIX, S3Settings::default())
    else {
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
    let Err(error) = FocusS3Destination::new(FakeS3Client::default(), PREFIX, settings) else {
        panic!("a destination with an empty bucket was built");
    };
    assert!(matches!(error, S3SettingsError::MissingBucketName));
}

#[test]
fn init_trims_trailing_slashes_from_the_prefix() {
    let (destination, _client) = s3_destination("exports///", s3_bucket_only_settings());
    assert_eq!(destination.prefix, "exports");
}

#[test]
fn init_trims_a_prefix_of_only_slashes() {
    let (destination, _client) = s3_destination("///", s3_bucket_only_settings());
    assert_eq!(destination.prefix, "");
}

#[test]
fn init_keeps_leading_slashes_in_the_prefix() {
    let (destination, _client) = s3_destination("/exports/", s3_bucket_only_settings());
    assert_eq!(destination.prefix, "/exports");
}

#[test]
fn init_accepts_a_bucket_without_client_options() {
    let (destination, _client) = s3_destination(PREFIX, s3_bucket_only_settings());
    assert_eq!(destination.bucket_name, BUCKET);
    assert!(destination.client_options.region_name.is_none());
    assert!(destination.client_options.endpoint_url.is_none());
    assert!(destination.client_options.aws_access_key_id.is_none());
    assert!(destination.client_options.aws_secret_access_key.is_none());
    assert!(destination.client_options.aws_session_token.is_none());
}

#[test]
fn init_accepts_every_optional_client_option() {
    let (destination, _client) = s3_destination(PREFIX, s3_settings());
    assert_eq!(destination.bucket_name, BUCKET);
    assert_eq!(
        destination.client_options.region_name.as_deref(),
        Some("us-east-1")
    );
    assert!(destination.client_options.aws_session_token.is_some());
}

#[tokio::test]
async fn deliver_uploads_the_export() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let puts = client.seen();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].body, CONTENT);
    assert_eq!(puts[0].content_type, CONTENT_TYPE);
}

#[tokio::test]
async fn deliver_awaits_the_async_upload() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    let delivery = destination.deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME);
    assert!(client.seen().is_empty());
    delivery.await.unwrap();
    assert_eq!(client.seen().len(), 1);
}

#[tokio::test]
async fn deliver_builds_the_daily_object_key() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        client.seen()[0].object_key,
        format!("{PREFIX}/date=2026-01-02/{FILENAME}")
    );
}

#[tokio::test]
async fn deliver_builds_the_hourly_object_key() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "hourly"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        client.seen()[0].object_key,
        format!("{PREFIX}/date=2026-01-02/hour=03/{FILENAME}")
    );
}

#[tokio::test]
async fn deliver_reports_upload_failures() {
    let destination =
        FocusS3Destination::new(FakeS3Client::failing(), PREFIX, s3_bucket_only_settings())
            .unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(error, S3DeliveryError::Upload { .. }));
}

#[tokio::test]
async fn deliver_uploads_every_delivery() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    for _ in 0..2 {
        destination
            .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
            .await
            .unwrap();
    }
    assert_eq!(client.seen().len(), 2);
}

#[tokio::test]
async fn deliver_uploads_empty_content() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(&[], &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(client.seen()[0].body.is_empty());
}

#[test]
fn object_key_daily_uses_the_date_partition() {
    let (destination, _client) = s3_destination(PREFIX, s3_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("{PREFIX}/date=2026-01-02/{FILENAME}"));
}

#[rstest]
#[case::midnight(1_767_312_000, "00")]
#[case::morning(1_767_323_045, "03")]
#[case::late(1_767_394_800, "23")]
#[test]
fn object_key_hourly_adds_a_zero_padded_hour(#[case] epoch: i64, #[case] hour: &str) {
    let (destination, _client) = s3_destination(PREFIX, s3_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(epoch, "hourly"), FILENAME);
    assert_eq!(
        key,
        format!("{PREFIX}/date=2026-01-02/hour={hour}/{FILENAME}")
    );
}

#[rstest]
#[case::daily("daily")]
#[case::interval("interval")]
#[case::all("all")]
#[case::uppercase("DAILY")]
#[test]
fn object_key_other_frequencies_omit_the_hour(#[case] frequency: &str) {
    let (destination, _client) = s3_destination(PREFIX, s3_bucket_only_settings());
    let key = object_key::build(
        &destination.prefix,
        &window(START_EPOCH, frequency),
        FILENAME,
    );
    assert_eq!(key, format!("{PREFIX}/date=2026-01-02/{FILENAME}"));
}

#[test]
fn object_key_empty_prefix_starts_with_the_date() {
    let (destination, _client) = s3_destination("", s3_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("date=2026-01-02/{FILENAME}"));
}

#[test]
fn object_key_prefix_trims_trailing_slashes() {
    let (destination, _client) = s3_destination("exports///", s3_bucket_only_settings());
    let key = object_key::build(
        &destination.prefix,
        &window(START_EPOCH, "hourly"),
        FILENAME,
    );
    assert_eq!(key, format!("exports/date=2026-01-02/hour=03/{FILENAME}"));
}

#[test]
fn object_key_keeps_a_leading_slash_prefix() {
    let (destination, _client) = s3_destination("/exports/", s3_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("/exports/date=2026-01-02/{FILENAME}"));
}

#[test]
fn kwargs_forwards_every_configured_client_option() {
    let (destination, _client) = s3_destination(PREFIX, s3_settings());
    let options = &destination.client_options;
    assert_eq!(options.region_name.as_deref(), Some("us-east-1"));
    assert_eq!(
        options.endpoint_url.as_deref(),
        Some("https://s3.example.test")
    );
    assert_eq!(
        options
            .aws_access_key_id
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some("access-key")
    );
    assert_eq!(
        options
            .aws_secret_access_key
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some("secret-key")
    );
    assert_eq!(
        options
            .aws_session_token
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some("session-token")
    );
}

#[test]
fn kwargs_omits_absent_client_options() {
    let (destination, _client) = s3_destination(PREFIX, s3_bucket_only_settings());
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
    let (destination, _client) = s3_destination(PREFIX, settings);
    let options = &destination.client_options;
    assert!(options.region_name.is_none());
    assert!(options.endpoint_url.is_none());
    assert!(options.aws_access_key_id.is_none());
    assert!(options.aws_secret_access_key.is_none());
    assert!(options.aws_session_token.is_none());
}

#[tokio::test]
async fn kwargs_reach_the_upload() {
    let (destination, client) = s3_destination(PREFIX, s3_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let puts = client.seen();
    let options = &puts[0].client_options;
    assert_eq!(options.region_name.as_deref(), Some("us-east-1"));
    assert_eq!(
        options.endpoint_url.as_deref(),
        Some("https://s3.example.test")
    );
    assert_eq!(
        options
            .aws_session_token
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some("session-token")
    );
}

#[tokio::test]
async fn upload_uses_the_octet_stream_content_type() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(client.seen()[0].content_type, "application/octet-stream");
}

#[tokio::test]
async fn upload_forwards_bucket_key_and_body() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let recorded = client.seen();
    let upload = &recorded[0];
    assert_eq!(upload.bucket_name, BUCKET);
    assert_eq!(
        upload.object_key,
        format!("{PREFIX}/date=2026-01-02/{FILENAME}")
    );
    assert_eq!(upload.body, CONTENT);
}

#[tokio::test]
async fn upload_surfaces_client_failures() {
    let destination =
        FocusS3Destination::new(FakeS3Client::failing(), PREFIX, s3_bucket_only_settings())
            .unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let S3DeliveryError::Upload { source } = error else {
        panic!("the upload failure was not reported");
    };
    assert!(matches!(source, NetworkDown));
}

#[tokio::test]
async fn upload_runs_once_per_delivery() {
    let (destination, client) = s3_destination(PREFIX, s3_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    destination
        .deliver(CONTENT, &window(START_EPOCH, "hourly"), FILENAME)
        .await
        .unwrap();
    assert_eq!(client.seen().len(), 2);
}
