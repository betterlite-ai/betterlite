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
    let observed = vec![transcript.frames[1].clone(), transcript.frames[0].clone()];
    assert!(direction_matches(&transcript.frames, &observed).is_ok());
    let different = vec![transcript.frames[0].clone()];
    assert_eq!(
        direction_matches(&transcript.frames, &different),
        Err("/provider/frames/0/text".to_owned())
    );
}

#[test]
fn barrier_gates_send() {
    let frame = Frame {
        direction: Direction::ClientToProvider,
        action: Action::Send,
        text: Some("followup".to_owned()),
        binary: None,
        binary_data: None,
        barrier: Some("after_provider_ready".to_owned()),
    };
    assert!(!barrier_is_released(&frame, &[]));
    assert!(barrier_is_released(
        &frame,
        &["after_provider_ready".to_owned()]
    ));
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
    let unknown = serde_json::json!({"service": "unknown", "input": null});
    assert!(serde_json::from_value::<crate::websocket_driver::ServiceInput>(unknown).is_err());
    assert_eq!(
        crate::websocket_driver::run_service(
            crate::websocket_driver::ServiceTag::RealtimeProbe,
            &serde_json::Value::Null,
        ),
        Err(crate::websocket_driver::DriverError::AdapterMissing(
            crate::websocket_driver::ServiceTag::RealtimeProbe
        ))
    );
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
    let close = Close {
        initiator: "provider".to_owned(),
        code: 1008,
        reason: "refused".to_owned(),
    };
    let changed_close = Close {
        code: 1011,
        ..close.clone()
    };
    assert_eq!(
        close_matches(Some(&close), Some(&changed_close)),
        Err("/close/code".to_owned())
    );
}

#[test]
fn bounded_custom_limits() {
    let limits = OracleLimits {
        max_frames: 2,
        max_frame_bytes: 4,
        max_capture_bytes: 6,
        max_execution_seconds: 3,
    };
    assert!(decode_ws_case(valid().as_bytes(), &limits).is_err());
    let short = valid().replace("setup", "a").replace("ready", "b");
    assert!(decode_ws_case(short.as_bytes(), &limits).is_ok());
    let binary_path = valid().replace("text: setup", "binary: ../outside.bin");
    assert!(decode_ws_case(binary_path.as_bytes(), &OracleLimits::default()).is_err());
}

#[test]
fn replays_corpus() {
    let seed = include_bytes!("../../fuzz/corpus/websocket_oracle/minimal.yaml");
    assert!(decode_ws_case(seed, &OracleLimits::default()).is_ok());
}
