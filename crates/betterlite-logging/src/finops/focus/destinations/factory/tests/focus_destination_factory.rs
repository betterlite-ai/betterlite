use std::io::Read as _;
use std::sync::atomic::Ordering;

use flate2::read::GzDecoder;
use http::Method as HttpMethod;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::factory::DestinationProvider;
use crate::finops::focus::destinations::factory::destination::DeliveryError;
use crate::finops::focus::destinations::factory::error::FactoryError;
use crate::finops::focus::destinations::factory::resolve::{self, DestinationOverrides};
use crate::finops::focus::destinations::focus_gcs_destination::GcsDeliveryError;
use crate::finops::focus::destinations::focus_s3_destination::S3DeliveryError;
use crate::finops::vantage::error::DeliveryError as VantageDeliveryError;
use crate::finops::vantage::settings::DEFAULT_BASE_URL;
use crate::focus_records::destinations::FocusDestination;
use crate::mavvrik_delivery::error::MavvrikDeliveryError;
use crate::mavvrik_delivery::transport::Method as MavvrikMethod;
use crate::mavvrik_delivery::validation::ValidationError;

use super::support::{
    ACCESS_KEY, BUCKET, MAVVRIK_CONNECTION, MAVVRIK_ENDPOINT, MAVVRIK_KEY, PREFIX, SECRET_KEY,
    SERVICE_ACCOUNT, SESSION_URI, SIGNED_URL, VANTAGE_KEY, VANTAGE_TOKEN, destination_overrides,
    environment, factory, factory_with_gcs_status, mavvrik_reply, recording_factory, time_window,
};

fn configured_secret(value: &Option<SecretString>) -> Option<&str> {
    value.as_ref().map(|secret| secret.expose_secret())
}

#[tokio::test]
async fn delivery_reaches_the_gcs_destination() {
    let subject = factory(&[("FOCUS_GCS_BUCKET_NAME", BUCKET)]);
    let destination = subject
        .create("gcs", PREFIX, &DestinationOverrides::default())
        .expect("the gcs provider is supported");
    let window = time_window();
    destination
        .deliver(b"ChargePeriodStart\n2026-01-02\n", &window, "usage.csv")
        .await
        .expect("the fake GCS upload answers 200");
}

#[tokio::test]
async fn delivery_reports_a_refused_gcs_upload() {
    let subject = factory_with_gcs_status(&[("FOCUS_GCS_BUCKET_NAME", BUCKET)], 500);
    let destination = subject
        .create("gcs", PREFIX, &DestinationOverrides::default())
        .expect("the gcs provider is supported");
    let window = time_window();
    let error = destination
        .deliver(b"ChargePeriodStart\n2026-01-02\n", &window, "usage.csv")
        .await
        .err()
        .expect("the refused upload is reported");
    assert!(matches!(
        error,
        DeliveryError::Gcs(GcsDeliveryError::Status { status: 500, .. })
    ));
}

#[tokio::test]
async fn delivery_reaches_the_s3_destination() {
    let (subject, recording) = recording_factory(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_ACCESS_KEY", ACCESS_KEY),
        ("FOCUS_S3_SECRET_KEY", SECRET_KEY),
    ]);
    let destination = subject
        .create("s3", PREFIX, &DestinationOverrides::default())
        .expect("the s3 provider is supported");
    destination
        .deliver(
            b"ChargePeriodStart\n2026-01-02\n",
            &time_window(),
            "usage.csv",
        )
        .await
        .expect("the fake S3 upload answers 200");
    let requests = recording.s3_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, HttpMethod::PUT);
    assert!(requests[0].url.contains("/exports/"));
    assert!(requests[0].url.ends_with("/usage.csv"));
    assert_eq!(requests[0].body, b"ChargePeriodStart\n2026-01-02\n");
}

#[tokio::test]
async fn delivery_reports_a_refused_s3_upload() {
    let (subject, recording) = recording_factory(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_ACCESS_KEY", ACCESS_KEY),
        ("FOCUS_S3_SECRET_KEY", SECRET_KEY),
    ]);
    recording.s3_status.store(503, Ordering::SeqCst);
    let destination = subject
        .create("s3", PREFIX, &DestinationOverrides::default())
        .expect("the s3 provider is supported");
    let error = destination
        .deliver(
            b"ChargePeriodStart\n2026-01-02\n",
            &time_window(),
            "usage.csv",
        )
        .await
        .err()
        .expect("the refused upload is reported");
    assert!(matches!(
        error,
        DeliveryError::S3(S3DeliveryError::Status { status: 503, .. })
    ));
    assert_eq!(recording.s3_requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delivery_reaches_the_vantage_destination() {
    let (subject, recording) = recording_factory(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ]);
    let destination = subject
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .expect("the vantage provider is supported");
    destination
        .deliver(b"ServiceName\nfoo\n", &time_window(), "usage.csv")
        .await
        .expect("the fake Vantage upload answers 200");
    let posts = recording.vantage_posts.lock().unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].filename.as_deref(), Some("usage.csv"));
    assert_eq!(posts[0].data, b"ServiceName\nfoo\n");
    assert!(posts[0].url.contains(VANTAGE_TOKEN));
}

#[tokio::test]
async fn delivery_reports_a_refused_vantage_upload() {
    let (subject, recording) = recording_factory(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ]);
    recording.vantage_status.store(500, Ordering::SeqCst);
    let destination = subject
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .expect("the vantage provider is supported");
    let error = destination
        .deliver(b"ServiceName\nfoo\n", &time_window(), "usage.csv")
        .await
        .err()
        .expect("the refused upload is reported");
    assert!(matches!(
        error,
        DeliveryError::Vantage(VantageDeliveryError::Status { status: 500 })
    ));
}

#[tokio::test]
async fn delivery_reaches_the_mavvrik_destination() {
    let (subject, recording) = recording_factory(&[
        ("MAVVRIK_API_KEY", MAVVRIK_KEY),
        ("MAVVRIK_API_ENDPOINT", MAVVRIK_ENDPOINT),
        ("MAVVRIK_CONNECTION_ID", MAVVRIK_CONNECTION),
    ]);
    {
        let mut replies = recording.mavvrik_replies.lock().unwrap();
        replies.push_back(mavvrik_reply(
            200,
            r#"{"metricsMarker": 1700000000}"#,
            Vec::new(),
        ));
        replies.push_back(mavvrik_reply(
            200,
            &format!(r#"{{"url": "{SIGNED_URL}"}}"#),
            Vec::new(),
        ));
        replies.push_back(mavvrik_reply(
            201,
            "",
            vec![("location".to_owned(), SESSION_URI.to_owned())],
        ));
        replies.push_back(mavvrik_reply(201, "", Vec::new()));
        replies.push_back(mavvrik_reply(200, "", Vec::new()));
    }
    let destination = subject
        .create("mavvrik", PREFIX, &DestinationOverrides::default())
        .expect("the mavvrik provider is supported");
    destination
        .deliver(
            b"ChargePeriodStart\n2026-01-02\n",
            &time_window(),
            "usage.csv",
        )
        .await
        .expect("the scripted Mavvrik delivery succeeds");
    let requests = recording.mavvrik_requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[0].method, MavvrikMethod::Post);
    assert_eq!(requests[1].method, MavvrikMethod::Get);
    assert_eq!(requests[2].method, MavvrikMethod::Post);
    assert_eq!(requests[3].method, MavvrikMethod::Put);
    assert_eq!(requests[3].url, SESSION_URI);
    assert_eq!(requests[4].method, MavvrikMethod::Patch);
    let mut uploaded = Vec::new();
    GzDecoder::new(requests[3].body.as_slice())
        .read_to_end(&mut uploaded)
        .expect("the upload body is gzip");
    assert_eq!(uploaded, b"ChargePeriodStart\n2026-01-02\n");
}

#[tokio::test]
async fn delivery_reports_a_refused_mavvrik_registration() {
    let (subject, recording) = recording_factory(&[
        ("MAVVRIK_API_KEY", MAVVRIK_KEY),
        ("MAVVRIK_API_ENDPOINT", MAVVRIK_ENDPOINT),
        ("MAVVRIK_CONNECTION_ID", MAVVRIK_CONNECTION),
    ]);
    recording
        .mavvrik_replies
        .lock()
        .unwrap()
        .push_back(mavvrik_reply(400, "the key is refused", Vec::new()));
    let destination = subject
        .create("mavvrik", PREFIX, &DestinationOverrides::default())
        .expect("the mavvrik provider is supported");
    let error = destination
        .deliver(
            b"ChargePeriodStart\n2026-01-02\n",
            &time_window(),
            "usage.csv",
        )
        .await
        .err()
        .expect("the refused registration is reported");
    assert!(matches!(
        error,
        DeliveryError::Mavvrik(MavvrikDeliveryError::RegisterFailed { status: 400, .. })
    ));
}

#[test]
fn validation_requires_an_s3_bucket_name() {
    let error = factory(&[])
        .create("s3", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the bucket is missing");
    assert_eq!(
        error.to_string(),
        "FOCUS_S3_BUCKET_NAME must be provided for S3 exports"
    );
    assert!(matches!(error, FactoryError::MissingS3BucketName));
}

#[test]
fn validation_uses_an_empty_bucket_override_as_given() {
    // docs/compatibility.md: an explicit empty value is used as given, while an empty env var counts as unset
    let mut overrides = destination_overrides();
    overrides.bucket_name = Some(String::new());
    let error = factory(&[("FOCUS_S3_BUCKET_NAME", BUCKET)])
        .create("s3", PREFIX, &overrides)
        .err()
        .expect("the empty bucket is refused");
    assert!(matches!(error, FactoryError::MissingS3BucketName));
}

#[test]
fn validation_requires_a_vantage_api_key() {
    let error = factory(&[("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN)])
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the api key is missing");
    assert_eq!(
        error.to_string(),
        "VANTAGE_API_KEY must be provided for Vantage exports"
    );
    assert!(matches!(error, FactoryError::MissingVantageApiKey));
}

#[test]
fn validation_rejects_an_empty_vantage_api_key_override() {
    // docs/compatibility.md: an explicit empty value is used as given, and an empty key names no key
    let mut overrides = destination_overrides();
    overrides.api_key = Some(SecretString::from(""));
    let error = factory(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ])
    .create("vantage", PREFIX, &overrides)
    .err()
    .expect("the empty api key is refused");
    assert!(matches!(error, FactoryError::MissingVantageApiKey));
}

#[test]
fn validation_requires_a_vantage_integration_token() {
    let error = factory(&[("VANTAGE_API_KEY", VANTAGE_KEY)])
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the integration token is missing");
    assert_eq!(
        error.to_string(),
        "VANTAGE_INTEGRATION_TOKEN must be provided for Vantage exports"
    );
    assert!(matches!(
        error,
        FactoryError::MissingVantageIntegrationToken
    ));
}

#[test]
fn validation_reports_the_missing_vantage_api_key_first() {
    let error = factory(&[])
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("both vantage values are missing");
    assert!(matches!(error, FactoryError::MissingVantageApiKey));
}

#[test]
fn validation_requires_a_gcs_bucket_name() {
    let error = factory(&[])
        .create("gcs", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the bucket is missing");
    assert_eq!(
        error.to_string(),
        "FOCUS_GCS_BUCKET_NAME must be provided for GCS exports"
    );
    assert!(matches!(error, FactoryError::MissingGcsBucketName));
}

#[test]
fn validation_leaves_the_mavvrik_settings_to_the_destination() {
    let error = factory(&[])
        .create("mavvrik", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the destination refuses the missing settings");
    assert!(matches!(
        error,
        FactoryError::MavvrikDestination(ValidationError::MissingSetting {
            key: "MAVVRIK_API_KEY"
        })
    ));
}

#[test]
fn config_prefers_the_s3_overrides_over_the_environment() {
    let variables = environment(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_REGION_NAME", "us-east-1"),
        ("FOCUS_S3_ENDPOINT_URL", "https://env.example.test"),
        ("FOCUS_S3_ACCESS_KEY", "env-access-key"),
        ("FOCUS_S3_SECRET_KEY", "env-secret-key"),
        ("FOCUS_S3_SESSION_TOKEN", "env-session-token"),
    ]);
    let mut overrides = destination_overrides();
    overrides.bucket_name = Some(String::from("override-bucket"));
    overrides.region_name = Some(String::from("eu-west-1"));
    overrides.endpoint_url = Some(String::from("https://override.example.test"));
    overrides.aws_access_key_id = Some(SecretString::from("override-access-key"));
    overrides.aws_secret_access_key = Some(SecretString::from("override-secret-key"));
    overrides.aws_session_token = Some(SecretString::from("override-session-token"));
    let settings =
        resolve::s3(&overrides, &variables).expect("the bucket is configured by the override");
    assert_eq!(settings.bucket_name.as_deref(), Some("override-bucket"));
    assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(
        settings.endpoint_url.as_deref(),
        Some("https://override.example.test")
    );
    assert_eq!(
        configured_secret(&settings.aws_access_key_id),
        Some("override-access-key")
    );
    assert_eq!(
        configured_secret(&settings.aws_secret_access_key),
        Some("override-secret-key")
    );
    assert_eq!(
        configured_secret(&settings.aws_session_token),
        Some("override-session-token")
    );
}

#[test]
fn config_reads_the_s3_environment() {
    let variables = environment(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_REGION_NAME", "us-east-1"),
        ("FOCUS_S3_ENDPOINT_URL", "https://env.example.test"),
        ("FOCUS_S3_ACCESS_KEY", ACCESS_KEY),
        ("FOCUS_S3_SECRET_KEY", SECRET_KEY),
        ("FOCUS_S3_SESSION_TOKEN", "session-token"),
    ]);
    let settings = resolve::s3(&DestinationOverrides::default(), &variables)
        .expect("the bucket is configured by the environment");
    assert_eq!(settings.bucket_name.as_deref(), Some(BUCKET));
    assert_eq!(settings.region_name.as_deref(), Some("us-east-1"));
    assert_eq!(
        settings.endpoint_url.as_deref(),
        Some("https://env.example.test")
    );
    assert_eq!(
        configured_secret(&settings.aws_access_key_id),
        Some(ACCESS_KEY)
    );
    assert_eq!(
        configured_secret(&settings.aws_secret_access_key),
        Some(SECRET_KEY)
    );
    assert_eq!(
        configured_secret(&settings.aws_session_token),
        Some("session-token")
    );
}

#[test]
fn config_leaves_absent_s3_values_unset() {
    let variables = environment(&[("FOCUS_S3_BUCKET_NAME", BUCKET)]);
    let settings = resolve::s3(&DestinationOverrides::default(), &variables)
        .expect("the bucket is configured by the environment");
    assert_eq!(settings.region_name, None);
    assert_eq!(settings.endpoint_url, None);
    assert!(settings.aws_access_key_id.is_none());
    assert!(settings.aws_secret_access_key.is_none());
    assert!(settings.aws_session_token.is_none());
}

#[test]
fn config_ignores_an_empty_s3_environment_value() {
    let variables = environment(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_REGION_NAME", ""),
    ]);
    let settings = resolve::s3(&DestinationOverrides::default(), &variables)
        .expect("the bucket is configured by the environment");
    assert_eq!(settings.region_name, None);
}

#[test]
fn config_uses_an_empty_s3_region_override_as_given() {
    // docs/compatibility.md: an explicit empty value is used as given, while an empty env var counts as unset
    let variables = environment(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_REGION_NAME", "us-east-1"),
    ]);
    let mut overrides = destination_overrides();
    overrides.region_name = Some(String::new());
    let settings = resolve::s3(&overrides, &variables).expect("the bucket is configured");
    assert_eq!(settings.region_name.as_deref(), Some(""));
}

#[test]
fn config_defaults_the_vantage_base_url() {
    let variables = environment(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ]);
    let settings = resolve::vantage(&DestinationOverrides::default(), &variables)
        .expect("the vantage settings are configured");
    assert_eq!(settings.base_url.as_deref(), Some(DEFAULT_BASE_URL));
    assert_eq!(configured_secret(&settings.api_key), Some(VANTAGE_KEY));
    assert_eq!(
        configured_secret(&settings.integration_token),
        Some(VANTAGE_TOKEN)
    );
}

#[test]
fn config_defaults_the_vantage_base_url_when_the_environment_is_empty() {
    let variables = environment(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
        ("VANTAGE_BASE_URL", ""),
    ]);
    let settings = resolve::vantage(&DestinationOverrides::default(), &variables)
        .expect("the vantage settings are configured");
    assert_eq!(settings.base_url.as_deref(), Some(DEFAULT_BASE_URL));
}

#[test]
fn config_prefers_the_vantage_overrides() {
    let variables = environment(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
        ("VANTAGE_BASE_URL", "https://env.example.test"),
    ]);
    let mut overrides = destination_overrides();
    overrides.api_key = Some(SecretString::from("override-key"));
    overrides.integration_token = Some(SecretString::from("override-token"));
    overrides.base_url = Some(String::from("https://override.example.test"));
    let settings =
        resolve::vantage(&overrides, &variables).expect("the vantage settings are configured");
    assert_eq!(configured_secret(&settings.api_key), Some("override-key"));
    assert_eq!(
        configured_secret(&settings.integration_token),
        Some("override-token")
    );
    assert_eq!(
        settings.base_url.as_deref(),
        Some("https://override.example.test")
    );
}

#[test]
fn config_reads_the_gcs_environment() {
    let variables = environment(&[
        ("FOCUS_GCS_BUCKET_NAME", BUCKET),
        ("FOCUS_GCS_PATH_SERVICE_ACCOUNT", SERVICE_ACCOUNT),
    ]);
    let settings = resolve::gcs(&DestinationOverrides::default(), &variables)
        .expect("the bucket is configured by the environment");
    assert_eq!(settings.bucket_name.as_deref(), Some(BUCKET));
    assert_eq!(
        configured_secret(&settings.service_account_json),
        Some(SERVICE_ACCOUNT)
    );
}

#[test]
fn config_prefers_the_gcs_override() {
    let variables = environment(&[
        ("FOCUS_GCS_BUCKET_NAME", BUCKET),
        ("FOCUS_GCS_PATH_SERVICE_ACCOUNT", SERVICE_ACCOUNT),
    ]);
    let mut overrides = destination_overrides();
    overrides.bucket_name = Some(String::from("override-bucket"));
    overrides.service_account_json = Some(SecretString::from("/secrets/override.json"));
    let settings = resolve::gcs(&overrides, &variables).expect("the gcs settings are configured");
    assert_eq!(settings.bucket_name.as_deref(), Some("override-bucket"));
    assert_eq!(
        configured_secret(&settings.service_account_json),
        Some("/secrets/override.json")
    );
}

#[test]
fn config_reads_the_mavvrik_environment() {
    let variables = environment(&[
        ("MAVVRIK_API_KEY", MAVVRIK_KEY),
        ("MAVVRIK_API_ENDPOINT", MAVVRIK_ENDPOINT),
        ("MAVVRIK_CONNECTION_ID", MAVVRIK_CONNECTION),
    ]);
    let settings = resolve::mavvrik(&DestinationOverrides::default(), &variables);
    assert_eq!(configured_secret(&settings.api_key), Some(MAVVRIK_KEY));
    assert_eq!(settings.api_endpoint.as_deref(), Some(MAVVRIK_ENDPOINT));
    assert_eq!(settings.connection_id.as_deref(), Some(MAVVRIK_CONNECTION));
}

#[test]
fn config_prefers_the_mavvrik_overrides() {
    let variables = environment(&[
        ("MAVVRIK_API_KEY", MAVVRIK_KEY),
        ("MAVVRIK_API_ENDPOINT", MAVVRIK_ENDPOINT),
        ("MAVVRIK_CONNECTION_ID", MAVVRIK_CONNECTION),
    ]);
    let mut overrides = destination_overrides();
    overrides.api_key = Some(SecretString::from("override-key"));
    overrides.api_endpoint = Some(String::from("https://override.example.test"));
    overrides.connection_id = Some(String::from("override-connection"));
    let settings = resolve::mavvrik(&overrides, &variables);
    assert_eq!(configured_secret(&settings.api_key), Some("override-key"));
    assert_eq!(
        settings.api_endpoint.as_deref(),
        Some("https://override.example.test")
    );
    assert_eq!(
        settings.connection_id.as_deref(),
        Some("override-connection")
    );
}

#[test]
fn config_leaves_absent_mavvrik_values_unset() {
    let variables = environment(&[]);
    let settings = resolve::mavvrik(&DestinationOverrides::default(), &variables);
    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_endpoint, None);
    assert_eq!(settings.connection_id, None);
}
