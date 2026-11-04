#![cfg(test)]

use std::path::Path;
use std::time::Duration;

use super::{
    Close, MAX_EXECUTION_SECONDS, OracleLimits, close_matches, decode_ws_case, direction_matches,
    validate_binary_fixture,
};
use crate::websocket_driver::{compare_transcript, exchange_gateway, start_provider};

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

#[rstest::rstest]
#[case::frame_count_boundary(1024, 1, 120, 0, true)]
#[case::frame_count_overflow(1025, 1, 120, 0, false)]
#[case::frame_size_boundary(1, 1_048_576, 120, 0, true)]
#[case::frame_size_overflow(1, 1_048_577, 120, 0, false)]
#[case::capture_boundary(16, 1_048_576, 120, 0, true)]
#[case::capture_overflow(16, 1_048_576, 120, 1, false)]
#[case::execution_overflow(1, 1, 121, 0, false)]
fn bounded_inputs(
    #[case] count: usize,
    #[case] size: usize,
    #[case] seconds: u64,
    #[case] extra: usize,
    #[case] accepted: bool,
) {
    let frame = format!(
        "  - direction: client_to_provider\n    action: send\n    text: {}\n",
        "x".repeat(size)
    );
    let header = valid().split("frames:").next().unwrap().to_owned();
    let mut input = format!(
        "{header}execution_seconds: {seconds}\nframes:\n{}",
        frame.repeat(count)
    );
    if extra != 0 {
        input.push_str(&format!(
            "  - direction: client_to_provider\n    action: send\n    text: {}\n",
            "x".repeat(extra)
        ));
    }
    assert_eq!(
        decode_ws_case(input.as_bytes(), &OracleLimits::default()).is_ok(),
        accepted
    );
    assert!(validate_binary_fixture(Path::new("../outside.bin")).is_err());
}

#[test]
fn direction_order() {
    let transcript = decode_ws_case(valid().as_bytes(), &OracleLimits::default()).unwrap();
    let setup = transcript.frames[0].clone();
    let ready = transcript.frames[1].clone();
    let mut followup = setup.clone();
    followup.text = Some("followup".to_owned());
    let mut delta = ready.clone();
    delta.text = Some("delta".to_owned());
    let expected = vec![
        ready.clone(),
        delta.clone(),
        setup.clone(),
        followup.clone(),
    ];
    let observed = vec![setup.clone(), ready, delta, followup];
    assert!(direction_matches(&expected, &observed).is_ok());
    assert_eq!(
        direction_matches(&transcript.frames, &[setup]),
        Err("/provider/frames/0/text".to_owned())
    );
}

#[tokio::test]
async fn barrier_gates_send() {
    let mut case = decode_ws_case(valid().as_bytes(), &OracleLimits::default()).unwrap();
    case.execution_seconds = 2;
    let mut followup = case.frames[0].clone();
    followup.text = Some("followup".to_owned());
    followup.barrier = Some("after_provider_ready".to_owned());
    case.frames[1].barrier = followup.barrier.clone();
    let mut delta = case.frames[1].clone();
    delta.text = Some("delta".to_owned());
    delta.barrier = None;
    case.frames.insert(1, followup);
    case.frames.push(delta);
    let mut script = case.clone();
    script.frames = vec![
        case.frames[0].clone(),
        case.frames[2].clone(),
        case.frames[3].clone(),
        case.frames[1].clone(),
    ];
    let provider = start_provider(script).await.unwrap();
    let observed = exchange_gateway(&format!("ws://{}", provider.address), &case)
        .await
        .unwrap();
    let upstream = provider.finish(Duration::from_secs(2)).await.unwrap();
    assert!(compare_transcript(&case, &observed).is_ok());
    assert_eq!(observed.frames[1].text.as_deref(), Some("ready"));
    assert_eq!(observed.frames[2].text.as_deref(), Some("followup"));
    assert!(direction_matches(&case.frames, &upstream.frames).is_ok());
}

#[tokio::test]
async fn preaccept_status() {
    let mut case = decode_ws_case(valid().as_bytes(), &OracleLimits::default()).unwrap();
    case.expected_http_status = 403;
    case.upstream_status = Some(403);
    case.execution_seconds = 2;
    let provider = start_provider(case.clone()).await.unwrap();
    let observed = exchange_gateway(&format!("ws://{}", provider.address), &case)
        .await
        .unwrap();
    let upstream = provider.finish(Duration::from_secs(2)).await.unwrap();
    assert_eq!(observed.handshake.status, 403);
    assert!(observed.close.is_none());
    assert!(observed.frames.is_empty());
    assert_eq!(upstream.connection_count, 1);
    let invalid = valid().replace("expected_http_status: 101", "expected_http_status: 403")
        + "close: {initiator: provider, code: 1008, reason: refused}\n";
    assert!(decode_ws_case(invalid.as_bytes(), &OracleLimits::default()).is_err());
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
    let output = crate::helpers::invoke(&crate::helpers::HelperCall {
        operation: "websocket_oracle".to_owned(),
        input: serde_json::json!({
            "service": "realtime_probe",
            "input": {"model": "realtime-a", "query_params": null, "api_base": "https://api.openai.com/"}
        }),
    });
    assert!(
        output.ok,
        "realtime service adapter must produce a transcript: {:?}",
        output.error
    );
}

#[test]
fn websocket_auth() {
    assert!(decode_ws_case(valid().as_bytes(), &OracleLimits::default()).is_ok());
}

#[test]
fn upstream_refusal() {
    assert_eq!(MAX_EXECUTION_SECONDS, 120);
}

#[rstest::rstest]
#[case::client(0, "/client/frames/0/text")]
#[case::provider(1, "/provider/frames/0/text")]
fn changed_peer(#[case] index: usize, #[case] pointer: &str) {
    let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default()).unwrap();
    let mut observed = case.frames.clone();
    observed[index].text = Some("changed".to_owned());
    assert_eq!(
        direction_matches(&case.frames, &observed),
        Err(pointer.to_owned())
    );
    let close = Close {
        initiator: "provider".to_owned(),
        code: 1008,
        reason: "refused".to_owned(),
    };
    let mut changed_close = close.clone();
    changed_close.code = 1011;
    assert_eq!(
        close_matches(Some(&close), Some(&changed_close)),
        Err("/close/code".to_owned())
    );
    changed_close.code = close.code;
    changed_close.reason = "changed".to_owned();
    assert_eq!(
        close_matches(Some(&close), Some(&changed_close)),
        Err("/close/reason".to_owned())
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/websocket_oracle");
    let outcomes = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            decode_ws_case(&bytes, &OracleLimits::default())
        })
        .collect::<Vec<_>>();
    assert!(!outcomes.is_empty());
}
