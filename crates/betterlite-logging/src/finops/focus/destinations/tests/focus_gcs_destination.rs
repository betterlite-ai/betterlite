use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::focus_gcs_destination::{
    FocusGcsDestination, GcsDeliveryError, GcsSettings, GcsSettingsError, MEDIA_UPLOAD_ROOT,
};
use crate::finops::focus::destinations::object_key;
use crate::finops::focus::destinations::tests::support::{
    BUCKET, CONTENT, ENVIRONMENT_SERVICE_ACCOUNT, FILENAME, FakeAuth, FakeGcsHttp, NetworkDown,
    PREFIX, SERVICE_ACCOUNT, START_EPOCH, TOKEN, gcs_base, gcs_bucket_only_settings,
    gcs_destination, gcs_settings, window,
};
use crate::focus_records::destinations::FocusDestination as _;
use crate::gcs_mock::client::GcsMockError;

fn service_account(destination: &FocusGcsDestination<FakeAuth, FakeGcsHttp>) -> Option<&str> {
    destination
        .service_account_json
        .as_ref()
        .map(|value| value.expose_secret().as_str())
}

#[test]
fn init_requires_a_bucket_name() {
    let base = gcs_base(FakeAuth::default(), FakeGcsHttp::answering(200, b""), &[]);
    let Err(error) = FocusGcsDestination::new(base, PREFIX, GcsSettings::default()) else {
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
    let base = gcs_base(FakeAuth::default(), FakeGcsHttp::answering(200, b""), &[]);
    let Err(error) = FocusGcsDestination::new(base, PREFIX, settings) else {
        panic!("a destination with an empty bucket was built");
    };
    assert!(matches!(error, GcsSettingsError::MissingBucketName));
}

#[rstest]
#[case::trailing("exports///", "exports")]
#[case::only_slashes("///", "")]
#[case::leading_kept("/exports/", "/exports")]
#[test]
fn init_trims_only_trailing_slashes_from_the_prefix(#[case] prefix: &str, #[case] expected: &str) {
    let (destination, _http) = gcs_destination(prefix, gcs_bucket_only_settings());
    assert_eq!(destination.prefix, expected);
}

#[test]
fn init_keeps_the_service_account_json() {
    let (destination, _http) = gcs_destination(PREFIX, gcs_settings());
    assert_eq!(service_account(&destination), Some(SERVICE_ACCOUNT));
}

#[test]
fn init_accepts_a_bucket_without_a_service_account() {
    let (destination, _http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    assert_eq!(destination.bucket_name, BUCKET);
    assert_eq!(service_account(&destination), None);
}

#[test]
fn init_defaults_the_service_account_to_the_environment_path() {
    let base = gcs_base(
        FakeAuth::default(),
        FakeGcsHttp::answering(200, b""),
        &[("GCS_PATH_SERVICE_ACCOUNT", ENVIRONMENT_SERVICE_ACCOUNT)],
    );
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    assert_eq!(
        service_account(&destination),
        Some(ENVIRONMENT_SERVICE_ACCOUNT)
    );
}

#[test]
fn init_config_service_account_overrides_the_environment_path() {
    let base = gcs_base(
        FakeAuth::default(),
        FakeGcsHttp::answering(200, b""),
        &[("GCS_PATH_SERVICE_ACCOUNT", ENVIRONMENT_SERVICE_ACCOUNT)],
    );
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_settings()).unwrap();
    assert_eq!(service_account(&destination), Some(SERVICE_ACCOUNT));
}

#[test]
fn init_keeps_an_empty_service_account_path() {
    let base = gcs_base(
        FakeAuth::default(),
        FakeGcsHttp::answering(200, b""),
        &[("GCS_PATH_SERVICE_ACCOUNT", ENVIRONMENT_SERVICE_ACCOUNT)],
    );
    let settings = GcsSettings {
        bucket_name: Some(BUCKET.to_owned()),
        service_account_json: Some(SecretString::from("")),
    };
    let destination = FocusGcsDestination::new(base, PREFIX, settings).unwrap();
    assert_eq!(service_account(&destination), Some(""));
}

#[tokio::test]
async fn deliver_requests_a_token_for_the_service_account() {
    let auth = FakeAuth::default();
    let base = gcs_base(auth.clone(), FakeGcsHttp::answering(200, b""), &[]);
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_settings()).unwrap();
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(auth.ensured(), vec![Some(SERVICE_ACCOUNT.to_owned())]);
}

#[tokio::test]
async fn deliver_omits_the_service_account_when_unset() {
    let auth = FakeAuth::default();
    let base = gcs_base(auth.clone(), FakeGcsHttp::answering(200, b""), &[]);
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(auth.ensured(), vec![None]);
}

#[tokio::test]
async fn deliver_uses_the_environment_service_account_when_unset() {
    let auth = FakeAuth::default();
    let base = gcs_base(
        auth.clone(),
        FakeGcsHttp::answering(200, b""),
        &[("GCS_PATH_SERVICE_ACCOUNT", ENVIRONMENT_SERVICE_ACCOUNT)],
    );
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        auth.ensured(),
        vec![Some(ENVIRONMENT_SERVICE_ACCOUNT.to_owned())]
    );
}

#[tokio::test]
async fn deliver_overwrites_the_content_type_header() {
    let (destination, http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let posts = http.posts();
    assert_eq!(
        posts[0].headers.get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        posts[0].headers.get("authorization").unwrap(),
        &format!("Bearer {TOKEN}")
    );
    assert!(
        posts[0]
            .headers
            .get("authorization")
            .unwrap()
            .is_sensitive()
    );
}

#[tokio::test]
async fn deliver_posts_the_encoded_object_name() {
    let (destination, http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        http.posts()[0].url,
        format!(
            "{MEDIA_UPLOAD_ROOT}/{BUCKET}/o?uploadType=media&name=exports%2Fdate%3D2026-01-02%2F{FILENAME}"
        )
    );
}

#[tokio::test]
async fn deliver_uploads_on_the_hourly_partition() {
    let (destination, http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "hourly"), FILENAME)
        .await
        .unwrap();
    assert_eq!(
        http.posts()[0].url,
        format!(
            "{MEDIA_UPLOAD_ROOT}/{BUCKET}/o?uploadType=media&name=exports%2Fdate%3D2026-01-02%2Fhour%3D03%2F{FILENAME}"
        )
    );
}

#[tokio::test]
async fn deliver_sends_the_export_bytes() {
    let (destination, http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    let posts = http.posts();
    assert_eq!(posts[0].content.as_deref(), Some(CONTENT));
    assert!(posts[0].data.is_none());
    assert!(posts[0].json.is_none());
}

#[tokio::test]
async fn deliver_uploads_empty_content() {
    let (destination, http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(&[], &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
    assert_eq!(http.posts()[0].content.as_deref(), Some(&[][..]));
}

#[tokio::test]
async fn deliver_surfaces_header_failures() {
    let http = FakeGcsHttp::answering(200, b"");
    let base = gcs_base(FakeAuth::failing(), http.clone(), &[]);
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(error, GcsDeliveryError::Headers(_)));
    assert!(http.posts().is_empty());
}

#[tokio::test]
async fn deliver_surfaces_transport_failures() {
    let base = gcs_base(FakeAuth::default(), FakeGcsHttp::failing(), &[]);
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GcsDeliveryError::Transport(GcsMockError::Transport {
            source: NetworkDown
        })
    ));
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
    let (destination, _http) = gcs_destination(prefix, gcs_bucket_only_settings());
    let key = object_key::build(&destination.prefix, &window(epoch, frequency), FILENAME);
    assert_eq!(key, format!("{directory}/{FILENAME}"));
}

#[tokio::test]
async fn status_200_is_accepted() {
    let (destination, _http) = gcs_destination(PREFIX, gcs_bucket_only_settings());
    destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap();
}

#[rstest]
#[case::created(201, b"", "")]
#[case::no_content(204, b"", "")]
#[case::forbidden(403, b"forbidden", "forbidden")]
#[case::server_error(500, b"", "")]
#[case::non_utf8_body(500, &[0xff, 0xfe], "")]
#[tokio::test]
async fn status_other_codes_are_rejected(
    #[case] status: u16,
    #[case] response: &[u8],
    #[case] expected_body: &str,
) {
    let base = gcs_base(
        FakeAuth::default(),
        FakeGcsHttp::answering(status, response),
        &[],
    );
    let destination = FocusGcsDestination::new(base, PREFIX, gcs_bucket_only_settings()).unwrap();
    let error = destination
        .deliver(CONTENT, &window(START_EPOCH, "daily"), FILENAME)
        .await
        .unwrap_err();
    let GcsDeliveryError::Status { status: seen, body } = error else {
        panic!("a status {status} response was not reported");
    };
    assert_eq!(seen, status);
    assert_eq!(body, expected_body);
}
