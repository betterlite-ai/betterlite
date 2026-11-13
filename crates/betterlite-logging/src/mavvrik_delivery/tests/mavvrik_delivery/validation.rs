use betterlite_logging::focus_records::destinations::FocusDestination as _;
use betterlite_logging::mavvrik_delivery::destination::{FocusMavvrikDestination, MavvrikSettings};
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;
use betterlite_logging::mavvrik_delivery::validation::{
    ValidationError, validate_api_endpoint, validate_gcs_url,
};
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    API_ROOT, ScriptedTransport, Step, block_on, destination, registered, respond,
    respond_with_location, settings, signed_url_found, window,
};

#[rstest]
#[case::dev("https://api.mavvrik.dev/tenant")]
#[case::ai("https://api.mavvrik.ai/tenant")]
#[case::app("https://api.mavvrik.app")]
#[case::uppercase_host("https://API.MAVVRIK.DEV/tenant")]
#[case::trailing_slash("https://api.mavvrik.dev/tenant/")]
fn accepts_mavvrik_endpoints(#[case] endpoint: &str) {
    validate_api_endpoint(endpoint).unwrap();
}

#[rstest]
#[case::http("http://api.mavvrik.dev/tenant", ValidationError::EndpointNotHttps)]
#[case::no_scheme("api.mavvrik.dev/tenant", ValidationError::EndpointNotHttps)]
#[case::other_host("https://example.com/tenant", ValidationError::EndpointNotMavvrik)]
#[case::bare_domain("https://mavvrik.dev/tenant", ValidationError::EndpointNotMavvrik)]
#[case::suffix_in_path(
    "https://example.com/.mavvrik.dev",
    ValidationError::EndpointNotMavvrik
)]
#[case::suffix_in_fragment(
    "https://example.com#.mavvrik.dev",
    ValidationError::EndpointNotMavvrik
)]
#[case::longer_domain(
    "https://api.mavvrik.dev.example.com",
    ValidationError::EndpointNotMavvrik
)]
#[case::unparsable("https://", ValidationError::EndpointNotMavvrik)]
fn rejects_other_endpoints(#[case] endpoint: &str, #[case] expected: ValidationError) {
    assert_eq!(validate_api_endpoint(endpoint).unwrap_err(), expected);
}

#[rstest]
#[case::http("http://api.mavvrik.dev/tenant", ValidationError::EndpointNotHttps)]
#[case::other_host("https://example.com/tenant", ValidationError::EndpointNotMavvrik)]
fn the_constructor_rejects_an_invalid_endpoint(
    #[case] endpoint: &str,
    #[case] expected: ValidationError,
) {
    let result = FocusMavvrikDestination::new(ScriptedTransport::default(), settings(endpoint));
    assert_eq!(result.unwrap_err(), expected);
}

#[rstest]
#[case::api_key(
    MavvrikSettings { api_key: None, ..settings(API_ROOT) },
    "MAVVRIK_API_KEY"
)]
#[case::empty_api_key(
    MavvrikSettings { api_key: Some(SecretString::from("")), ..settings(API_ROOT) },
    "MAVVRIK_API_KEY"
)]
#[case::api_endpoint(
    MavvrikSettings { api_endpoint: None, ..settings(API_ROOT) },
    "MAVVRIK_API_ENDPOINT"
)]
#[case::empty_api_endpoint(
    MavvrikSettings { api_endpoint: Some(String::new()), ..settings(API_ROOT) },
    "MAVVRIK_API_ENDPOINT"
)]
#[case::connection_id(
    MavvrikSettings { connection_id: None, ..settings(API_ROOT) },
    "MAVVRIK_CONNECTION_ID"
)]
#[case::empty_connection_id(
    MavvrikSettings { connection_id: Some(String::new()), ..settings(API_ROOT) },
    "MAVVRIK_CONNECTION_ID"
)]
fn a_missing_setting_names_its_key(#[case] given: MavvrikSettings, #[case] key: &'static str) {
    let error = FocusMavvrikDestination::new(ScriptedTransport::default(), given).unwrap_err();
    assert_eq!(error, ValidationError::MissingSetting { key });
    assert_eq!(
        error.to_string(),
        format!(
            "{key} must be provided for Mavvrik FOCUS destination (set {key} env var or pass in destination_config)"
        )
    );
}

#[rstest]
#[case::bucket_host("https://storage.googleapis.com/bucket/object?sig=1")]
#[case::subdomain("https://bucket.storage.googleapis.com/object")]
#[case::uppercase("https://STORAGE.GOOGLEAPIS.COM/bucket")]
fn accepts_gcs_urls(#[case] url: &str) {
    validate_gcs_url(url, "signed URL").unwrap();
}

#[rstest]
#[case::http("http://storage.googleapis.com/bucket", "GcsUrlNotHttps")]
#[case::other_host("https://example.com/bucket", "GcsUrlNotStorage")]
#[case::suffix_trick(
    "https://storage.googleapis.com.example.com/bucket",
    "GcsUrlNotStorage"
)]
#[case::prefix_trick("https://evilstorage.googleapis.com/bucket", "GcsUrlNotStorage")]
#[case::userinfo_trick(
    "https://storage.googleapis.com@example.com/bucket",
    "GcsUrlNotStorage"
)]
#[case::not_a_url("not a url", "GcsUrlUnparsable")]
fn rejects_other_gcs_urls(#[case] url: &str, #[case] variant: &str) {
    let error = validate_gcs_url(url, "signed URL").unwrap_err();
    assert!(format!("{error:?}").starts_with(variant), "{error:?}");
}

#[rstest]
#[case::signed_url_http(
    vec![registered(), respond(200, r#"{"url": "http://storage.googleapis.com/x"}"#)],
    vec![Method::Post, Method::Get]
)]
#[case::signed_url_foreign_host(
    vec![registered(), respond(200, r#"{"url": "https://example.com/x"}"#)],
    vec![Method::Post, Method::Get]
)]
#[case::session_uri_http(
    vec![registered(), signed_url_found(), respond_with_location(200, "http://storage.googleapis.com/session")],
    vec![Method::Post, Method::Get, Method::Post]
)]
#[case::session_uri_foreign_host(
    vec![registered(), signed_url_found(), respond_with_location(200, "https://example.com/session")],
    vec![Method::Post, Method::Get, Method::Post]
)]
fn a_url_outside_gcs_is_rejected_before_any_data_is_sent(
    #[case] steps: Vec<Step>,
    #[case] expected_calls: Vec<Method>,
) {
    let (destination, transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::Validation(_)),
        "{error:?}"
    );
    assert_eq!(transport.methods(), expected_calls);
}
