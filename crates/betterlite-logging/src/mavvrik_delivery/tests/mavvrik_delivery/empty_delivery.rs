use betterlite_logging::focus_records::destinations::FocusDestination as _;
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;

use super::support::{AGENT_URL, WINDOW_EPOCH, block_on, destination, registered, respond, window};

#[test]
fn empty_content_advances_the_marker_without_any_upload_call() {
    let (destination, transport) = destination(vec![registered(), respond(200, "")]);
    block_on(destination.deliver(b"", &window(), "export.csv")).unwrap();
    let calls = transport.drain();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        (calls[0].method, calls[0].url.as_str()),
        (Method::Post, AGENT_URL)
    );
    assert_eq!(
        (calls[1].method, calls[1].url.as_str()),
        (Method::Patch, AGENT_URL)
    );
    assert_eq!(
        calls[1].body,
        format!(r#"{{"metricsMarker":{WINDOW_EPOCH}}}"#).as_bytes()
    );
}

#[test]
fn an_empty_delivery_after_registration_only_patches_the_marker() {
    let (destination, transport) =
        destination(vec![registered(), respond(200, ""), respond(200, "")]);
    block_on(destination.deliver(b"", &window(), "one.csv")).unwrap();
    transport.drain();
    block_on(destination.deliver(b"", &window(), "two.csv")).unwrap();
    assert_eq!(transport.methods(), [Method::Patch]);
}

#[test]
fn a_failed_marker_update_for_empty_content_is_reported() {
    let (destination, _transport) =
        destination(vec![registered(), respond(500, "marker store down")]);
    let error = block_on(destination.deliver(b"", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(
            &error,
            MavvrikDeliveryError::MarkerUpdateFailed { status: 500, body } if body == "marker store down"
        ),
        "{error:?}"
    );
}

#[test]
fn a_gone_marker_update_for_empty_content_resets_registration() {
    let (destination, transport) = destination(vec![
        registered(),
        respond(410, ""),
        registered(),
        respond(200, ""),
    ]);
    let error = block_on(destination.deliver(b"", &window(), "one.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::Disconnected),
        "{error:?}"
    );
    block_on(destination.deliver(b"", &window(), "two.csv")).unwrap();
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Patch, Method::Post, Method::Patch]
    );
}
