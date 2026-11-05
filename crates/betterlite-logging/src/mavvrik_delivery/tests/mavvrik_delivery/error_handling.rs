use betterlite_logging::focus_records::destinations::FocusDestination as _;
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;
use rstest::rstest;

use super::support::{
    SESSION_URI, ScriptedError, Step, block_on, destination, noise, registered, respond,
    respond_with_location, signed_url_found, window,
};

fn session_started() -> Vec<Step> {
    vec![
        registered(),
        signed_url_found(),
        respond_with_location(200, SESSION_URI),
    ]
}

#[rstest]
#[case::server_error(respond(500, "storage down"))]
#[case::wrong_status(respond(204, ""))]
fn a_failed_chunk_cancels_the_session_and_keeps_the_original_error(#[case] chunk: Step) {
    let mut steps = session_started();
    steps.extend([chunk, respond(204, "")]);
    let (destination, transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::ChunkFailed { .. }),
        "{error:?}"
    );

    let calls = transport.drain();
    assert_eq!(
        calls.iter().map(|call| call.method).collect::<Vec<_>>(),
        [
            Method::Post,
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Delete
        ]
    );
    let cancel = &calls[4];
    assert_eq!(cancel.url, SESSION_URI);
    assert_eq!(cancel.timeout_secs, 10);
    assert!(cancel.body.is_empty());
}

#[test]
fn a_chunk_timeout_cancels_the_session_and_surfaces_the_timeout() {
    let mut steps = session_started();
    steps.extend([Step::Timeout, respond(204, "")]);
    let (destination, transport) = destination(steps);
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
    assert_eq!(transport.methods().last(), Some(&Method::Delete));
}

#[rstest]
#[case::timeout(Step::Timeout)]
#[case::refused(respond(403, "denied"))]
fn a_failed_cancellation_does_not_mask_the_chunk_error(#[case] cancel: Step) {
    let mut steps = session_started();
    steps.extend([respond(500, "storage down"), cancel]);
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::ChunkFailed { status: 500, body, .. } if body == "storage down"),
        "{error:?}"
    );
}

#[test]
fn a_failure_after_an_accepted_chunk_cancels_but_the_accepted_chunk_does_not() {
    let content = noise(9 * 1024 * 1024);
    let mut steps = session_started();
    steps.extend([
        respond(308, ""),
        respond(500, "storage down"),
        respond(204, ""),
    ]);
    let (destination, transport) = destination(steps);
    let error = block_on(destination.deliver(&content, &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::ChunkFailed { .. }),
        "{error:?}"
    );
    let methods = transport.methods();
    assert_eq!(&methods[3..], [Method::Put, Method::Put, Method::Delete]);
}

#[test]
fn a_successful_upload_never_cancels_the_session() {
    let content = noise(9 * 1024 * 1024);
    let mut steps = session_started();
    steps.extend([respond(308, ""), respond(200, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(&content, &window(), "export.csv")).unwrap();
    assert!(!transport.methods().contains(&Method::Delete));
}

#[rstest]
#[case::signed_url_failure(vec![registered(), respond(500, "")])]
#[case::session_init_failure(vec![registered(), signed_url_found(), respond(500, "")])]
#[case::missing_location(vec![registered(), signed_url_found(), respond(200, "")])]
fn a_failure_before_a_session_exists_has_nothing_to_cancel(#[case] steps: Vec<Step>) {
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(!transport.methods().contains(&Method::Delete));
}

#[test]
fn a_gone_marker_update_after_an_upload_resets_registration() {
    let mut steps = session_started();
    steps.extend([respond(200, ""), respond(410, ""), registered()]);
    steps.extend([signed_url_found(), respond_with_location(200, SESSION_URI)]);
    steps.extend([respond(200, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "one.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::Disconnected),
        "{error:?}"
    );
    block_on(destination.deliver(b"data", &window(), "two.csv")).unwrap();
    let methods = transport.methods();
    assert_eq!(
        methods
            .iter()
            .filter(|method| **method == Method::Post)
            .count(),
        4
    );
}

#[test]
fn a_marker_update_error_after_an_upload_reports_status_and_body() {
    let mut steps = session_started();
    steps.extend([respond(200, ""), respond(500, "marker store down")]);
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::MarkerUpdateFailed { status: 500, body } if body == "marker store down"),
        "{error:?}"
    );
}
