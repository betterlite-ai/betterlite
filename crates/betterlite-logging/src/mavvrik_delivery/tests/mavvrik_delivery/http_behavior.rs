use betterlite_logging::focus_records::destinations::{FocusDestination as _, FocusTimeWindow};
use betterlite_logging::mavvrik_delivery::destination::FocusMavvrikDestination;
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;
use chrono::DateTime;
use rstest::rstest;

use super::support::{
    AGENT_URL, SESSION_URI, SIGNED_URL, ScriptedError, ScriptedTransport, Step, WINDOW_EPOCH,
    block_on, destination, registered, respond, respond_with_location, settings, signed_url_found,
    window,
};

fn upload_flow() -> Vec<Step> {
    vec![
        registered(),
        signed_url_found(),
        respond_with_location(200, SESSION_URI),
        respond(200, ""),
        respond(200, ""),
    ]
}

fn failed_body(error: &MavvrikDeliveryError<ScriptedError>) -> (u16, String) {
    let (MavvrikDeliveryError::RegisterFailed { status, body }
    | MavvrikDeliveryError::SignedUrlFailed { status, body }
    | MavvrikDeliveryError::MarkerUpdateFailed { status, body }) = error
    else {
        panic!("not an API status error: {error:?}");
    };
    (*status, body.clone())
}

#[test]
fn registration_posts_the_connection_name_with_the_api_key_and_a_30_second_deadline() {
    let (destination, transport) = destination(vec![registered(), respond(200, "{}")]);
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    let calls = transport.drain();
    let register = &calls[0];
    assert_eq!(register.method, Method::Post);
    assert_eq!(register.url, AGENT_URL);
    assert_eq!(register.body, br#"{"name":"conn-1"}"#);
    assert_eq!(register.api_key.as_deref(), Some("key-123"));
    assert!(
        register
            .headers
            .contains(&("Content-Type", "application/json".to_owned()))
    );
    assert_eq!(register.timeout_secs, 30);
}

#[test]
fn the_upload_url_request_carries_the_date_and_a_30_second_deadline() {
    let (destination, transport) = destination(upload_flow());
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap();
    let calls = transport.drain();
    let upload_url = &calls[1];
    assert_eq!(upload_url.method, Method::Get);
    assert_eq!(
        upload_url.url,
        format!("{AGENT_URL}/upload-url?name=2024-03-05&type=metrics&datetime=2024-03-05")
    );
    assert_eq!(upload_url.api_key.as_deref(), Some("key-123"));
    assert_eq!(upload_url.timeout_secs, 30);
}

#[test]
fn the_marker_patch_sends_the_window_start_epoch_with_a_30_second_deadline() {
    let (destination, transport) = destination(vec![registered(), respond(200, "")]);
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    let calls = transport.drain();
    let patch = &calls[1];
    assert_eq!(patch.method, Method::Patch);
    assert_eq!(patch.url, AGENT_URL);
    assert_eq!(
        patch.body,
        format!(r#"{{"metricsMarker":{WINDOW_EPOCH}}}"#).as_bytes()
    );
    assert_eq!(patch.api_key.as_deref(), Some("key-123"));
    assert_eq!(patch.timeout_secs, 30);
}

#[test]
fn the_date_key_and_marker_come_from_the_window_start_in_utc() {
    let late = 1_709_681_400;
    let window = FocusTimeWindow {
        start_time: DateTime::from_timestamp(late, 0).unwrap(),
        end_time: DateTime::from_timestamp(late + 3600, 0).unwrap(),
        frequency: "hourly".to_owned(),
    };
    let (destination, transport) = destination(vec![registered(), respond(200, "")]);
    block_on(destination.deliver(b"", &window, "export.csv")).unwrap();
    let patch = &transport.drain()[1];
    assert_eq!(
        patch.body,
        format!(r#"{{"metricsMarker":{late}}}"#).as_bytes()
    );
}

#[test]
fn the_endpoint_trailing_slashes_are_dropped_from_every_url() {
    let transport = ScriptedTransport::new(vec![registered()]);
    let destination = FocusMavvrikDestination::new(
        transport.clone(),
        settings("https://api.mavvrik.dev/tenant///"),
    )
    .unwrap();
    block_on(destination.get_metrics_marker()).unwrap();
    assert_eq!(transport.drain()[0].url, AGENT_URL);
}

#[rstest]
#[case::register_client_error(vec![respond(400, &"a".repeat(300))], 400)]
#[case::register_server_error(vec![respond(503, &"a".repeat(300))], 503)]
#[case::upload_url_client_error(vec![registered(), respond(403, &"a".repeat(300))], 403)]
#[case::upload_url_server_error(vec![registered(), respond(500, &"a".repeat(300))], 500)]
fn api_errors_carry_the_status_and_the_first_200_bytes(
    #[case] steps: Vec<Step>,
    #[case] status: u16,
) {
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert_eq!(failed_body(&error), (status, "a".repeat(200)));
}

#[rstest]
#[case::client_error(404)]
#[case::server_error(502)]
fn marker_errors_carry_the_status_and_the_first_200_bytes(#[case] status: u16) {
    let (destination, _transport) =
        destination(vec![registered(), respond(status, &"b".repeat(300))]);
    let error = block_on(destination.deliver(b"", &window(), "export.csv")).unwrap_err();
    assert_eq!(failed_body(&error), (status, "b".repeat(200)));
}

#[test]
fn a_short_error_body_is_kept_whole() {
    let (destination, _transport) = destination(vec![respond(400, "bad connection")]);
    let error = block_on(destination.deliver(b"", &window(), "export.csv")).unwrap_err();
    assert_eq!(failed_body(&error), (400, "bad connection".to_owned()));
}

#[test]
fn the_error_prefix_never_splits_a_multibyte_character() {
    let (destination, _transport) = destination(vec![respond(400, &"\u{e9}".repeat(150))]);
    let error = block_on(destination.deliver(b"", &window(), "export.csv")).unwrap_err();
    assert_eq!(failed_body(&error), (400, "\u{e9}".repeat(100)));
}

#[test]
fn status_399_is_a_success_and_400_is_an_error() {
    let (destination, _transport) = destination(vec![respond(399, "{}"), respond(400, "")]);
    let error = block_on(destination.deliver(b"", &window(), "export.csv")).unwrap_err();
    assert_eq!(failed_body(&error), (400, String::new()));
}

#[rstest]
#[case::register(vec![Step::Timeout])]
#[case::upload_url(vec![registered(), Step::Timeout])]
#[case::session_init(vec![registered(), signed_url_found(), Step::Timeout])]
#[case::marker(vec![registered(), signed_url_found(), respond_with_location(200, SESSION_URI), respond(200, ""), Step::Timeout])]
fn a_transport_timeout_surfaces_as_the_transport_error_without_a_response(
    #[case] steps: Vec<Step>,
) {
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(
            error,
            MavvrikDeliveryError::Transport {
                source: ScriptedError::Timeout
            }
        ),
        "{error:?}"
    );
}

#[test]
fn registration_is_cached_for_later_deliveries_of_the_same_instance() {
    let (destination, transport) = destination(vec![
        registered(),
        respond(200, ""),
        respond(200, ""),
        respond(200, ""),
    ]);
    block_on(destination.deliver(b"", &window(), "one.csv")).unwrap();
    block_on(destination.deliver(b"", &window(), "two.csv")).unwrap();
    block_on(destination.deliver(b"", &window(), "three.csv")).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Patch, Method::Patch, Method::Patch]
    );
}

#[test]
fn a_failed_registration_is_retried_by_the_next_delivery() {
    let (destination, transport) =
        destination(vec![respond(500, ""), registered(), respond(200, "")]);
    block_on(destination.deliver(b"", &window(), "one.csv")).unwrap_err();
    block_on(destination.deliver(b"", &window(), "two.csv")).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Post, Method::Patch]
    );
}

#[test]
fn a_gone_registration_reports_the_disconnect_and_registers_again_next_time() {
    let (destination, transport) =
        destination(vec![respond(410, ""), registered(), respond(200, "")]);
    let error = block_on(destination.deliver(b"", &window(), "one.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::Disconnected),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains("Re-enable the connection in the Mavvrik dashboard")
    );
    block_on(destination.deliver(b"", &window(), "two.csv")).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Post, Method::Patch]
    );
}

#[test]
fn the_signed_url_is_what_the_session_is_started_on() {
    let (destination, transport) = destination(upload_flow());
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap();
    assert_eq!(transport.drain()[2].url, SIGNED_URL);
}

#[rstest]
#[case::missing_url("{}")]
#[case::null_url(r#"{"url": null}"#)]
#[case::empty_url(r#"{"url": ""}"#)]
fn an_upload_url_response_without_a_url_is_an_error(#[case] body: &str) {
    let (destination, _transport) = destination(vec![registered(), respond(200, body)]);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::SignedUrlMissing),
        "{error:?}"
    );
}
