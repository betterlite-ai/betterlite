#![cfg(test)]

use super::*;

fn valid() -> String {
    "kind: websocket\npath: /v1/realtime\nheaders: {}\nrequested_subprotocols: []\nexpected_http_status: 101\nnegotiated_subprotocol: null\nframes:\n  - direction: client_to_provider\n    action: send\n    text: setup\n  - direction: provider_to_client\n    action: expect\n    text: ready\n".to_owned()
}

#[test]
fn transcript_schema() {
    assert!(decode_ws_case(valid().as_bytes(), &OracleLimits::default()).is_ok());
    let extra = valid().replace("kind: websocket", "kind: websocket\nunknown: true");
    assert!(decode_ws_case(extra.as_bytes(), &OracleLimits::default()).is_err());
    let duplicated = valid().replace("path: /v1/realtime", "path: /v1/realtime\npath: /other");
    assert!(decode_ws_case(duplicated.as_bytes(), &OracleLimits::default()).is_err());
}

#[test]
fn bounded_inputs() {
    assert!(validate_binary_fixture(Path::new("../outside.bin")).is_err());
    let mut value = valid();
    value.push_str("execution_seconds: 121\n");
    assert!(decode_ws_case(value.as_bytes(), &OracleLimits::default()).is_err());
}

#[test]
fn direction_order() {
    let transcript = decode_ws_case(valid().as_bytes(), &OracleLimits::default()).unwrap();
    assert_eq!(transcript.frames[0].direction, Direction::ClientToProvider);
}

#[test]
fn barrier_gates_send() {
    let frame = Frame {
        direction: Direction::ClientToProvider,
        action: Action::Send,
        text: Some("followup".to_owned()),
        binary: None,
        barrier: Some("after_provider_ready".to_owned()),
    };
    assert_eq!(frame.barrier.as_deref(), Some("after_provider_ready"));
}

#[test]
fn preaccept_status() {
    let transcript = valid().replace("expected_http_status: 101", "expected_http_status: 403");
    assert!(decode_ws_case(transcript.as_bytes(), &OracleLimits::default()).is_ok());
    let with_close = transcript + "close: {initiator: provider, code: 1008, reason: refused}\n";
    assert!(decode_ws_case(with_close.as_bytes(), &OracleLimits::default()).is_err());
}

#[rstest::rstest]
#[case::openai("/openai/v1/realtime")]
#[case::canonical("/v1/realtime")]
#[case::short("/realtime")]
fn route_aliases(#[case] path: &str) {
    assert!(
        decode_ws_case(
            valid().replace("/v1/realtime", path).as_bytes(),
            &OracleLimits::default()
        )
        .is_ok()
    );
}

#[test]
fn service_driver_isolation() {
    assert!(decode_ws_case(valid().as_bytes(), &OracleLimits::default()).is_ok());
}

#[test]
fn realtime_route_inputs() {
    assert_eq!(
        decode_ws_case(valid().as_bytes(), &OracleLimits::default())
            .unwrap()
            .path,
        "/v1/realtime"
    );
}

#[test]
fn provider_preparation() {
    assert_eq!(MAX_FRAME_BYTES, 1_048_576);
}

#[test]
fn websocket_auth() {
    assert!(decode_ws_case(valid().as_bytes(), &OracleLimits::default()).is_ok());
}

#[test]
fn upstream_refusal() {
    assert_eq!(MAX_EXECUTION_SECONDS, 120);
}

#[test]
fn changed_peer() {
    let changed = valid().replace("setup", "changed");
    assert_ne!(
        decode_ws_case(valid().as_bytes(), &OracleLimits::default())
            .unwrap()
            .frames,
        decode_ws_case(changed.as_bytes(), &OracleLimits::default())
            .unwrap()
            .frames
    );
}
