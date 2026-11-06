use betterlite_logging::focus_records::destinations::FocusDestination as _;
use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;
use rstest::rstest;

use super::support::{AGENT_URL, block_on, destination, registered, respond, window};

#[test]
fn get_metrics_marker_posts_the_registration_and_returns_the_marker() {
    let (destination, transport) = destination(vec![registered()]);
    let marker = block_on(destination.get_metrics_marker()).unwrap();
    assert_eq!(marker, MetricsMarker::Epoch(1_700_000_000));
    let calls = transport.drain();
    assert_eq!(
        (calls[0].method, calls[0].url.as_str()),
        (Method::Post, AGENT_URL)
    );
    assert_eq!(calls[0].body, br#"{"name":"conn-1"}"#);
}

#[test]
fn get_metrics_marker_posts_again_on_every_call() {
    let (destination, transport) = destination(vec![registered(), registered(), registered()]);
    block_on(destination.get_metrics_marker()).unwrap();
    block_on(destination.get_metrics_marker()).unwrap();
    block_on(destination.get_metrics_marker()).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Post, Method::Post]
    );
}

#[test]
fn get_metrics_marker_marks_the_instance_registered_for_deliver() {
    let (destination, transport) = destination(vec![registered(), respond(200, "")]);
    block_on(destination.get_metrics_marker()).unwrap();
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    assert_eq!(transport.methods(), [Method::Post, Method::Patch]);
}

#[test]
fn deliver_after_a_get_metrics_marker_call_does_not_register_again_but_the_next_get_does() {
    let (destination, transport) = destination(vec![registered(), respond(200, ""), registered()]);
    block_on(destination.get_metrics_marker()).unwrap();
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    block_on(destination.get_metrics_marker()).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Patch, Method::Post]
    );
}

#[rstest]
#[case::integer(
    r#"{"metricsMarker": 1700000000}"#,
    MetricsMarker::Epoch(1_700_000_000)
)]
#[case::zero(r#"{"metricsMarker": 0}"#, MetricsMarker::Epoch(0))]
#[case::text(r#"{"metricsMarker": "2024-03-05"}"#, MetricsMarker::Text("2024-03-05".to_owned()))]
#[case::empty_text(r#"{"metricsMarker": ""}"#, MetricsMarker::Text(String::new()))]
#[case::missing("{}", MetricsMarker::Epoch(0))]
#[case::null(r#"{"metricsMarker": null}"#, MetricsMarker::Epoch(0))]
fn the_marker_field_is_read_and_defaults_to_zero(
    #[case] body: &str,
    #[case] expected: MetricsMarker,
) {
    let (destination, _transport) = destination(vec![respond(200, body)]);
    assert_eq!(
        block_on(destination.get_metrics_marker()).unwrap(),
        expected
    );
}

#[test]
fn a_gone_connector_resets_registration_for_the_next_delivery() {
    let (destination, transport) = destination(vec![
        registered(),
        respond(410, ""),
        registered(),
        respond(200, ""),
    ]);
    block_on(destination.get_metrics_marker()).unwrap();
    let error = block_on(destination.get_metrics_marker()).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::Disconnected),
        "{error:?}"
    );
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Post, Method::Post, Method::Patch]
    );
}

#[test]
fn a_failed_registration_reports_the_status_and_body() {
    let (destination, _transport) = destination(vec![respond(401, "bad key")]);
    let error = block_on(destination.get_metrics_marker()).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::RegisterFailed { status: 401, body } if body == "bad key"),
        "{error:?}"
    );
}

#[test]
fn a_registration_body_that_is_not_json_is_an_error() {
    let (destination, _transport) = destination(vec![respond(200, "not json")]);
    let error = block_on(destination.get_metrics_marker()).unwrap_err();
    assert!(matches!(error, MavvrikDeliveryError::Json(_)), "{error:?}");
}
