use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::focus_gcs_destination::{
    FocusGcsDestination, GcsDeliveryError, GcsSettings, GcsSettingsError, MEDIA_CONTENT_TYPE,
    MEDIA_UPLOAD_ROOT,
};
use crate::finops::focus::destinations::object_key;
use crate::finops::focus::destinations::tests::support::{
    BUCKET, CONTENT, FILENAME, FakeGcsClient, GcsAnswer, PREFIX, SERVICE_ACCOUNT, START_EPOCH,
    gcs_bucket_only_settings, gcs_destination, gcs_settings, window,
};
use crate::focus_records::destinations::FocusDestination as _;
use crate::gcs_client::gcs_storage_auth::CONTENT_TYPE_HEADER;

#[test]
fn init_requires_a_bucket_name() {
    let Err(error) =
        FocusGcsDestination::new(FakeGcsClient::default(), PREFIX, GcsSettings::default())
    else {
        panic!("a destination without a bucket was built");
    };
    assert!(matches!(error, GcsSettingsError::MissingBucketName));
}

#[test]
fn init_rejects_an_empty_bucket_name() {
    let settings = GcsSettings {
        bucket_name: Some(String::new()),
        service_account_json: None,
    };
    let Err(error) = FocusGcsDestination::new(FakeGcsClient::default(), PREFIX, settings) else {
        panic!("a destination with an empty bucket was built");
    };
    assert!(matches!(error, GcsSettingsError::MissingBucketName));
}

#[test]
fn init_trims_trailing_slashes_from_the_prefix() {
    let (destination, _client) = gcs_destination("exports///", gcs_bucket_only_settings());
    assert_eq!(destination.prefix, "exports");
}

#[test]
fn init_trims_a_prefix_of_only_slashes() {
    let (destination, _client) = gcs_destination("///", gcs_bucket_only_settings());
    assert_eq!(destination.prefix, "");
}

#[test]
fn init_keeps_leading_slashes_in_the_prefix() {
    let (destination, _client) = gcs_destination("/exports/", gcs_bucket_only_settings());
    assert_eq!(destination.prefix, "/exports");
}

#[test]
fn init_keeps_the_service_account_json() {
    let (destination, _client) = gcs_destination(PREFIX, gcs_settings());
    assert_eq!(
        destination
            .service_account_json
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some(SERVICE_ACCOUNT)
    );
}

#[test]
fn init_accepts_a_bucket_without_a_service_account() {
    let (destination, _client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    assert_eq!(destination.bucket_name, BUCKET);
    assert!(destination.service_account_json.is_none());
}

#[test]
fn init_keeps_an_empty_service_account_path() {
    let settings = GcsSettings {
        bucket_name: Some(BUCKET.to_owned()),
        service_account_json: Some(SecretString::from("")),
    };
    let (destination, _client) = gcs_destination(PREFIX, settings);
    assert_eq!(
        destination
            .service_account_json
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some("")
    );
}

#[tokio::test]
async fn deliver_requests_headers_for_the_bucket() {
    let (destination, client) = gcs_destination(PREFIX, gcs_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let calls = client.header_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].bucket_name, BUCKET);
    assert_eq!(
        calls[0].service_account_json.as_deref(),
        Some(SERVICE_ACCOUNT)
    );
}

#[tokio::test]
async fn deliver_omits_the_service_account_when_unset() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(client.header_calls()[0].service_account_json, None);
}

#[tokio::test]
async fn deliver_overwrites_the_content_type_header() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let posts = client.posts();
    let content_type = posts[0]
        .headers
        .get(CONTENT_TYPE_HEADER)
        .map(|value| value.expose_secret().as_str());
    assert_eq!(content_type, Some(MEDIA_CONTENT_TYPE));
    let authorization = posts[0]
        .headers
        .get("Authorization")
        .map(|value| value.expose_secret().as_str());
    assert_eq!(authorization, Some("Bearer test-token"));
}

#[tokio::test]
async fn deliver_posts_the_encoded_object_name() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        client.posts()[0].url,
        format!(
            "{MEDIA_UPLOAD_ROOT}/{BUCKET}/o?uploadType=media&name=exports%2Fdate%3D2026-01-02%2F{FILENAME}"
        )
    );
}

#[tokio::test]
async fn deliver_uploads_on_the_hourly_partition() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "hourly"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        client.posts()[0].url,
        format!(
            "{MEDIA_UPLOAD_ROOT}/{BUCKET}/o?uploadType=media&name=exports%2Fdate%3D2026-01-02%2Fhour%3D03%2F{FILENAME}"
        )
    );
}

#[tokio::test]
async fn deliver_sends_the_export_bytes() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(client.posts()[0].body, CONTENT);
}

#[tokio::test]
async fn deliver_uploads_empty_content() {
    let (destination, client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(&[], &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert!(client.posts()[0].body.is_empty());
}

#[tokio::test]
async fn deliver_surfaces_header_failures() {
    let client = FakeGcsClient::answering(GcsAnswer::HeadersFailure);
    let destination =
        FocusGcsDestination::new(client.clone(), PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(error, GcsDeliveryError::Headers { .. }));
    assert_eq!(client.header_calls().len(), 1);
    assert!(client.posts().is_empty());
}

#[tokio::test]
async fn deliver_surfaces_transport_failures() {
    let destination = FocusGcsDestination::new(
        FakeGcsClient::answering(GcsAnswer::PostFailure),
        PREFIX,
        gcs_bucket_only_settings(),
    )
    .unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(error, GcsDeliveryError::Transport { .. }));
}

#[test]
fn object_key_daily_uses_the_date_partition() {
    let (destination, _client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("{PREFIX}/date=2026-01-02/{FILENAME}"));
}

#[rstest]
#[case::midnight(1_767_312_000, "00")]
#[case::morning(1_767_323_045, "03")]
#[case::late(1_767_394_800, "23")]
#[test]
fn object_key_hourly_adds_a_zero_padded_hour(#[case] epoch: i64, #[case] hour: &str) {
    let (destination, _client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
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
    let (destination, _client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    let key = object_key::build(
        &destination.prefix,
        &window(START_EPOCH, frequency),
        FILENAME,
    );
    assert_eq!(key, format!("{PREFIX}/date=2026-01-02/{FILENAME}"));
}

#[test]
fn object_key_empty_prefix_starts_with_the_date() {
    let (destination, _client) = gcs_destination("", gcs_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("date=2026-01-02/{FILENAME}"));
}

#[test]
fn object_key_prefix_trims_trailing_slashes() {
    let (destination, _client) = gcs_destination("exports///", gcs_bucket_only_settings());
    let key = object_key::build(
        &destination.prefix,
        &window(START_EPOCH, "hourly"),
        FILENAME,
    );
    assert_eq!(key, format!("exports/date=2026-01-02/hour=03/{FILENAME}"));
}

#[test]
fn object_key_keeps_a_leading_slash_prefix() {
    let (destination, _client) = gcs_destination("/exports/", gcs_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(START_EPOCH, "daily"), FILENAME);
    assert_eq!(key, format!("/exports/date=2026-01-02/{FILENAME}"));
}

#[tokio::test]
async fn status_200_is_accepted() {
    let (destination, _client) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::not_found(404)]
#[case::server_error(500)]
#[tokio::test]
async fn status_other_codes_are_rejected(#[case] status: u16) {
    let client = FakeGcsClient::answering(GcsAnswer::Reply {
        status,
        body: Vec::new(),
    });
    let destination = FocusGcsDestination::new(client, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let GcsDeliveryError::Status { status: seen, .. } = error else {
        panic!("a status {status} response was not reported");
    };
    assert_eq!(seen, status);
}

#[tokio::test]
async fn status_error_carries_the_response_body() {
    let client = FakeGcsClient::answering(GcsAnswer::Reply {
        status: 403,
        body: b"forbidden".to_vec(),
    });
    let destination = FocusGcsDestination::new(client, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let GcsDeliveryError::Status { status, body } = error else {
        panic!("the refused upload was not reported");
    };
    assert_eq!(status, 403);
    assert_eq!(body, "forbidden");
}

#[tokio::test]
async fn status_error_with_a_non_utf8_body_keeps_an_empty_message() {
    let client = FakeGcsClient::answering(GcsAnswer::Reply {
        status: 500,
        body: vec![0xff, 0xfe],
    });
    let destination = FocusGcsDestination::new(client, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let GcsDeliveryError::Status { body, .. } = error else {
        panic!("the refused upload was not reported");
    };
    assert!(body.is_empty());
}
