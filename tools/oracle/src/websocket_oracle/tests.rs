#![cfg(test)]

use crate::process::{Gateway, GatewaySpec};
use std::fmt::Write as _;
use std::fs;
use std::future::pending;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::time::timeout;

use secrecy::{ExposeSecret as _, SecretString};

use super::decode::{FilesystemFixtures, decode_ws_case, validate_binary_fixture};
use super::{
    Action, AuthObservation, Close, Direction, Frame, Initiator, OracleLimits, RealtimeSettings,
    RouteObservation, WebSocketBounds, WebSocketCase, close_matches, direction_matches,
    route_auth_matches,
};
use crate::websocket_driver::gateway::{TokioTransport, exchange as exchange_gateway};
use crate::websocket_driver::provider::{ProviderObservation, start as start_provider};
use crate::websocket_driver::service::{Arguments, ServiceInput, ServiceTag, Services};
use crate::websocket_driver::{DriverError, TokioClock, TranscriptObservation, compare_transcript};

fn limits() -> OracleLimits {
    OracleLimits::default()
}

fn settings() -> RealtimeSettings {
    RealtimeSettings::default()
}

fn bounds() -> WebSocketBounds {
    WebSocketBounds {
        limits: limits(),
        settings: settings(),
    }
}

fn fixtures() -> FilesystemFixtures {
    FilesystemFixtures {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/websocket-oracle"),
    }
}

fn valid() -> String {
    "kind: websocket\npath: /v1/realtime\nheaders: {}\nrequested_subprotocols: []\nexpected_http_status: 101\nnegotiated_subprotocol: null\nframes:\n  - direction: client_to_provider\n    action: send\n    text: setup\n  - direction: provider_to_client\n    action: expect\n    text: ready\n".to_owned()
}

#[rstest::rstest]
#[case::headers("headers: {}\n")]
#[case::protocols("requested_subprotocols: []\n")]
#[case::negotiation("negotiated_subprotocol: null\n")]
#[tokio::test]
async fn transcript_schema(#[case] required: &str) {
    let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    matching_peer(case).await;
    let missing = valid().replace(required, "");
    decode_ws_case(missing.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
    let extra = valid().replace("kind: websocket", "kind: websocket\nunknown: true");
    decode_ws_case(extra.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
    let duplicated = valid().replace("path: /v1/realtime", "path: /v1/realtime\npath: /other");
    decode_ws_case(duplicated.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
}

#[rstest::rstest]
#[case::frame_count_boundary(1024, 1, 120, 0, true)]
#[case::frame_count_overflow(1025, 1, 120, 0, false)]
#[case::frame_size_boundary(1, 0x0010_0000, 120, 0, true)]
#[case::frame_size_overflow(1, 0x0010_0001, 120, 0, false)]
#[case::capture_boundary(16, 0x0010_0000, 120, 0, true)]
#[case::capture_overflow(16, 0x0010_0000, 120, 1, false)]
#[case::execution_overflow(1, 1, 121, 0, false)]
#[tokio::test]
async fn bounded_inputs(
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
        write!(
            input,
            "  - direction: client_to_provider\n    action: send\n    text: {}\n",
            "x".repeat(extra)
        )
        .unwrap();
    }
    assert_eq!(
        decode_ws_case(input.as_bytes(), &OracleLimits::default(), &fixtures()).is_ok(),
        accepted
    );
    validate_binary_fixture(Path::new("../outside.bin")).unwrap_err();
    let smuggled = valid().replace(
        "text: setup",
        "text: setup\n    binary_data: [1, 2, 3, 4, 5]",
    );
    decode_ws_case(smuggled.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
    let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    matching_peer(case).await;
    if count == 1024 {
        bounded_child().await;
    }
}

#[tokio::test]
async fn direction_order() {
    let transcript =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    matching_peer(transcript.clone()).await;
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
    direction_matches(&expected, &observed).unwrap();
    assert_eq!(
        direction_matches(&transcript.frames, &[setup]),
        Err("/provider/frames/0/text".to_owned())
    );
}

#[tokio::test]
async fn barrier_gates_send() {
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
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
    let provider = start_provider(case.clone(), limits(), settings(), &TokioClock)
        .await
        .unwrap();
    let observed = exchange_gateway(
        &format!("ws://{}", provider.address),
        &case,
        &bounds(),
        &TokioClock,
        &TokioTransport,
    )
    .await
    .unwrap();
    let upstream = provider.finish(Duration::from_secs(2)).await.unwrap();
    compare_transcript(&case, &observed).unwrap();
    assert_eq!(observed.frames[1].text.as_deref(), Some("ready"));
    assert_eq!(observed.frames[2].text.as_deref(), Some("followup"));
    direction_matches(&case.frames, &upstream.frames).unwrap();
    let mut changed = observed;
    changed.frames[1].text = Some("changed".to_owned());
    assert_eq!(
        compare_transcript(&case, &changed),
        Err("/provider/frames/0/text".to_owned())
    );
}

#[tokio::test]
async fn preaccept_status() {
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.expected_http_status = 403;
    case.upstream_status = Some(403);
    case.execution_seconds = 2;
    let provider = start_provider(case.clone(), limits(), settings(), &TokioClock)
        .await
        .unwrap();
    let observed = exchange_gateway(
        &format!("ws://{}", provider.address),
        &case,
        &bounds(),
        &TokioClock,
        &TokioTransport,
    )
    .await
    .unwrap();
    let upstream = provider.finish(Duration::from_secs(2)).await.unwrap();
    assert_eq!(observed.handshake.status, 403);
    assert!(observed.close.is_none());
    assert!(observed.frames.is_empty());
    assert_eq!(upstream.connection_count, 1);
    case.frames.clear();
    matching_peer(case).await;
    let mut invalid = valid().replace("expected_http_status: 101", "expected_http_status: 403");
    invalid.push_str("close: {initiator: provider, code: 1008, reason: refused}\n");
    decode_ws_case(invalid.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
}

#[rstest::rstest]
#[case::openai("/openai/v1/realtime")]
#[case::canonical("/v1/realtime")]
#[case::short("/realtime")]
#[tokio::test]
async fn route_aliases(#[case] path: &str) {
    let case = decode_ws_case(
        valid().replace("/v1/realtime", path).as_bytes(),
        &OracleLimits::default(),
        &fixtures(),
    )
    .unwrap();
    matching_peer(case).await;
}

#[rstest::rstest]
#[case::native(ServiceTag::NativeResponsesRelay, "native")]
#[case::managed(ServiceTag::ManagedResponsesTurns, "managed")]
#[case::realtime(ServiceTag::RealtimeProbe, "probe")]
fn service_driver_isolation(#[case] tag: ServiceTag, #[case] response: &str) {
    let mut registry = Services::default();
    assert!(matches!(
        registry.run_service(tag, &Arguments::default()),
        Err(DriverError::AdapterMissing(_))
    ));
    registry
        .register_service(ServiceTag::NativeResponsesRelay, |args| {
            adapter(args, "native")
        })
        .unwrap();
    registry
        .register_service(ServiceTag::ManagedResponsesTurns, |args| {
            adapter(args, "managed")
        })
        .unwrap();
    registry
        .register_service(ServiceTag::RealtimeProbe, |args| adapter(args, "probe"))
        .unwrap();
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.frames[1].text = Some(response.to_owned());
    let defaults = Arguments::default();
    let args = Arguments {
        frames: case.frames.clone(),
        ..defaults
    };
    let mut observed = registry.run_service(tag, &args).unwrap();
    compare_transcript(&case, &observed).unwrap();
    observed.frames[1].text = Some("changed".to_owned());
    assert_eq!(
        compare_transcript(&case, &observed),
        Err("/provider/frames/0/text".to_owned())
    );
    serde_json::from_str::<ServiceInput>(r#"{"service":"unknown","input":{}}"#).unwrap_err();
    let mut invalid = valid();
    invalid.push_str("service: realtime_probe\n");
    decode_ws_case(invalid.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap_err();
}

fn adapter(args: &Arguments, response: &str) -> Result<TranscriptObservation, DriverError> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures())?;
        case.frames = args.frames.clone();
        let mut script = case.clone();
        script.frames[1].text = Some(response.to_owned());
        run_scripted_exchange(case, script).await
    })
}

async fn run_scripted_exchange(
    case: WebSocketCase,
    script: WebSocketCase,
) -> Result<TranscriptObservation, DriverError> {
    let provider = start_provider(script, limits(), settings(), &TokioClock).await?;
    let result = exchange_gateway(
        &format!("ws://{}", provider.address),
        &case,
        &bounds(),
        &TokioClock,
        &TokioTransport,
    )
    .await?;
    let upstream = provider.finish(Duration::from_secs(2)).await?;
    if upstream.connection_count != 1 {
        return Err(DriverError::Transport(
            "unexpected connection count".to_owned(),
        ));
    }
    Ok(result)
}

#[rstest::rstest]
#[case::model("/realtime?model=realtime-a", None, "realtime-a", &[])]
#[case::transcription(
    "/v1/realtime?intent=transcription",
    None,
    "gpt-realtime-whisper",
    &[]
)]
#[case::guardrails(
    "/openai/v1/realtime?model=realtime-a&guardrails=%20first,%20,second%20",
    Some("realtime"),
    "realtime-a",
    &["first", "second"]
)]
#[tokio::test]
async fn realtime_route_inputs(
    #[case] path: &str,
    #[case] protocol: Option<&str>,
    #[case] model: &str,
    #[case] guardrails: &[&str],
) {
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.path = path.to_owned();
    case.negotiated_subprotocol = protocol.map(str::to_owned);
    case.requested_subprotocols =
        protocol.map_or_else(Vec::new, |name| vec![name.to_owned(), "second".to_owned()]);
    case.route_model = Some(model.to_owned());
    case.route_guardrails = guardrails.iter().map(|&value| value.to_owned()).collect();
    matching_peer(case).await;
}

#[rstest::rstest]
#[case::absent("null", "null")]
#[case::empty("{}", "[]")]
#[case::ordered(
    r#"{"model":"realtime-a","intent":"transcription"}"#,
    r#"[["model","realtime-a"],["intent","transcription"]]"#
)]
fn provider_preparation(#[case] query: &str, #[case] wire: &str) {
    let raw = format!(
        r#"{{"service":"realtime_probe","input":{{"model":"realtime-a","query_params":{query}}}}}"#
    );
    let input: ServiceInput = serde_json::from_str(&raw).unwrap();
    let encoded = serde_json::to_value(input.to_wire()).unwrap();
    let query_json =
        serde_json::to_string(encoded.pointer("/input/query_params").unwrap()).unwrap();
    assert_eq!(query_json, wire);
    let restored: ServiceInput = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored.input.query_params, input.input.query_params);
    let mut registry = Services::default();
    assert!(matches!(
        registry.run_service(input.service, &input.input),
        Err(DriverError::AdapterMissing(ServiceTag::RealtimeProbe))
    ));
    registry
        .register_service(ServiceTag::RealtimeProbe, |args| adapter(args, "probe"))
        .unwrap();
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.frames[1].text = Some("probe".to_owned());
    let args = Arguments {
        frames: case.frames.clone(),
        ..input.input.clone()
    };
    let mut observed = registry.run_service(input.service, &args).unwrap();
    compare_transcript(&case, &observed).unwrap();
    observed.frames[1].text = Some("changed".to_owned());
    assert_eq!(
        compare_transcript(&case, &observed),
        Err("/provider/frames/0/text".to_owned())
    );
}

fn provider_frame(text: &str) -> Frame {
    Frame {
        direction: Direction::ProviderToClient,
        action: Action::Expect,
        text: Some(text.to_owned()),
        binary: None,
        binary_data: None,
        barrier: None,
    }
}

fn client_frame() -> Frame {
    Frame {
        direction: Direction::ClientToProvider,
        action: Action::Send,
        text: Some("setup".to_owned()),
        binary: None,
        binary_data: None,
        barrier: None,
    }
}

fn prepare_realtime(args: &Arguments) -> Result<[String; 3], DriverError> {
    let key = args.api_key.as_ref().ok_or_else(|| {
        DriverError::Transport("api_key is required for OpenAI realtime calls".to_owned())
    })?;
    let query = args
        .query_params
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let base = args
        .api_base
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let url = if query.is_empty() {
        format!("{base}v1/realtime")
    } else {
        format!("{base}v1/realtime?{query}")
    };
    let mut headers = format!("Authorization: Bearer {}", key.expose_secret());
    if args.openai_beta_realtime {
        headers.push_str("\nOpenAI-Beta: realtime=v1");
    }
    Ok([url, query, headers])
}

fn realtime_preparation_adapter(args: &Arguments) -> Result<TranscriptObservation, DriverError> {
    let responses = prepare_realtime(args)?;
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures())?;
        let mut expected = case;
        expected.frames = args.frames.clone();
        let mut script = expected.clone();
        for (frame, response) in script
            .frames
            .iter_mut()
            .filter(|frame| frame.direction == Direction::ProviderToClient)
            .zip(&responses)
        {
            frame.text = Some(response.clone());
        }
        run_scripted_exchange(expected, script).await
    })
}

fn preparation_args(
    query_params: Option<&[(&str, &str)]>,
    openai_beta_realtime: bool,
    url: &str,
    query: &str,
    headers: &str,
) -> Arguments {
    Arguments {
        model: "realtime-a".to_owned(),
        api_base: "https://api.openai.com/".to_owned(),
        api_key: Some(SecretString::from("sk-test")),
        query_params: query_params.map(|pairs| {
            pairs
                .iter()
                .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                .collect()
        }),
        openai_beta_realtime,
        frames: vec![
            client_frame(),
            provider_frame(url),
            provider_frame(query),
            provider_frame(headers),
        ],
    }
}

#[rstest::rstest]
#[case::default_url(
    None,
    false,
    "wss://api.openai.com/v1/realtime",
    "",
    "Authorization: Bearer sk-test"
)]
#[case::query_params(
    Some(&[("intent", "transcription")][..]),
    false,
    "wss://api.openai.com/v1/realtime?intent=transcription",
    "intent=transcription",
    "Authorization: Bearer sk-test"
)]
#[case::beta_header(
    None,
    true,
    "wss://api.openai.com/v1/realtime",
    "",
    "Authorization: Bearer sk-test\nOpenAI-Beta: realtime=v1"
)]
fn provider_preparation_probe(
    #[case] query_params: Option<&[(&str, &str)]>,
    #[case] openai_beta_realtime: bool,
    #[case] url: &str,
    #[case] query: &str,
    #[case] headers: &str,
) {
    let mut registry = Services::default();
    registry
        .register_service(ServiceTag::RealtimeProbe, realtime_preparation_adapter)
        .unwrap();
    let args = preparation_args(query_params, openai_beta_realtime, url, query, headers);
    let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    let mut expected = case;
    expected.frames = args.frames.clone();
    let observed = registry
        .run_service(ServiceTag::RealtimeProbe, &args)
        .unwrap();
    compare_transcript(&expected, &observed).unwrap();
    let provider_frames: Vec<usize> = observed
        .frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.direction == Direction::ProviderToClient)
        .map(|(index, _)| index)
        .collect();
    for (frame_index, pointer) in [
        "/provider/frames/0/text",
        "/provider/frames/1/text",
        "/provider/frames/2/text",
    ]
    .into_iter()
    .enumerate()
    {
        let mut mismatched = observed.clone();
        mismatched.frames[provider_frames[frame_index]].text = Some("changed".to_owned());
        assert_eq!(
            compare_transcript(&expected, &mismatched),
            Err(pointer.to_owned())
        );
    }
}

#[test]
fn provider_preparation_missing_key() {
    let mut registry = Services::default();
    registry
        .register_service(ServiceTag::RealtimeProbe, realtime_preparation_adapter)
        .unwrap();
    let args = Arguments {
        model: "realtime-a".to_owned(),
        api_base: "https://api.openai.com/".to_owned(),
        api_key: None,
        query_params: None,
        openai_beta_realtime: false,
        frames: Vec::new(),
    };
    assert!(matches!(
        registry.run_service(ServiceTag::RealtimeProbe, &args),
        Err(DriverError::Transport(message))
            if message == "api_key is required for OpenAI realtime calls"
    ));
}

#[rstest::rstest]
#[case::bearer(Some(("Authorization", "Bearer sk-test")), 101, "authorization_bearer")]
#[case::api_key(Some(("api-key", "sk-test")), 101, "api_key_header")]
#[case::missing(None, 403, "missing")]
#[case::invalid(Some(("Authorization", "Basic wrong")), 403, "invalid_authorization_format")]
#[case::subprotocol(
    Some(("sec-websocket-protocol", "realtime, openai-insecure-api-key.sk-test")),
    101,
    "subprotocol"
)]
#[tokio::test]
async fn websocket_auth(
    #[case] header: Option<(&str, &str)>,
    #[case] status: u16,
    #[case] detail: &str,
) {
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.expected_http_status = status;
    if status != 101 {
        case.frames.clear();
    }
    if let Some((name, value)) = header {
        case.headers.insert(name.to_owned(), value.to_owned());
        if name == "sec-websocket-protocol" {
            case.negotiated_subprotocol = Some("realtime".to_owned());
        }
    }
    case.authentication_detail = Some(detail.to_owned());
    case.attempted_close_code = (status != 101).then_some(1008);
    matching_peer(case).await;
}

#[rstest::rstest]
#[case::unauthorized(401, 1008)]
#[case::forbidden(403, 1008)]
#[case::rate_limit(429, 1013)]
#[case::unavailable(503, 1011)]
#[tokio::test]
async fn upstream_refusal(#[case] status: u16, #[case] code: u16) {
    let mut case =
        decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    case.frames.remove(0);
    let message = format!("Upstream realtime handshake rejected with HTTP {status}");
    case.frames[0].text = Some(format!(
        r#"{{"type":"error","error":{{"type":"server_error","message":"{message}"}}}}"#
    ));
    case.close = Some(Close {
        initiator: Initiator::Provider,
        code,
        reason: message,
    });
    matching_peer(case).await;
}

async fn matching_peer(case: WebSocketCase) {
    let mut script = case.clone();
    script.upstream_status = Some(case.expected_http_status);
    let peer = start_provider(script, limits(), settings(), &TokioClock)
        .await
        .unwrap();
    let mut observed = exchange_gateway(
        &format!("ws://{}", peer.address),
        &case,
        &bounds(),
        &TokioClock,
        &TokioTransport,
    )
    .await
    .unwrap();
    let capture = peer.finish(Duration::from_secs(2)).await.unwrap();
    assert_eq!(capture.connection_count, 1);
    assert_eq!(capture.request_uri, case.path);
    assert_route_auth_mismatches(&case, &capture);
    compare_transcript(&case, &observed).unwrap();
    if let Some(close) = observed.close.as_mut() {
        close.reason = "changed".to_owned();
        assert_eq!(
            compare_transcript(&case, &observed),
            Err("/close/reason".to_owned())
        );
    } else {
        observed.handshake.status = 418;
        assert_eq!(
            compare_transcript(&case, &observed),
            Err("/handshake/status".to_owned())
        );
    }
}

type RouteAuthMutation = fn(&mut RouteObservation, &mut AuthObservation);

fn route_auth_mutations() -> [(RouteAuthMutation, &'static str); 4] {
    [
        (
            |route, _| route.model = Some("changed".to_owned()),
            "/route/model",
        ),
        (
            |route, _| route.guardrails.push("changed".to_owned()),
            "/route/guardrails",
        ),
        (
            |_, auth| auth.detail = Some("changed".to_owned()),
            "/auth/detail",
        ),
        (
            |_, auth| auth.attempted_close_code = Some(4000),
            "/auth/attempted_close_code",
        ),
    ]
}

fn assert_route_auth_mismatches(case: &WebSocketCase, capture: &ProviderObservation) {
    let expected_route = RouteObservation {
        model: case.route_model.clone(),
        guardrails: case.route_guardrails.clone(),
    };
    let expected_auth = AuthObservation {
        detail: case.authentication_detail.clone(),
        attempted_close_code: case.attempted_close_code,
    };
    route_auth_matches(
        &expected_route,
        &capture.route,
        &expected_auth,
        &capture.auth,
    )
    .unwrap();
    for (mutate, pointer) in route_auth_mutations() {
        let mut route = capture.route.clone();
        let mut auth = capture.auth.clone();
        mutate(&mut route, &mut auth);
        assert_eq!(
            route_auth_matches(&expected_route, &route, &expected_auth, &auth),
            Err(pointer.to_owned())
        );
    }
}

#[rstest::rstest]
#[case::client(0, "/client/frames/0/text")]
#[case::provider(1, "/provider/frames/0/text")]
#[tokio::test]
async fn changed_peer(#[case] index: usize, #[case] pointer: &str) {
    let case = decode_ws_case(valid().as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    matching_peer(case.clone()).await;
    let mut observed = case.frames.clone();
    observed[index].text = Some("changed".to_owned());
    assert_eq!(
        direction_matches(&case.frames, &observed),
        Err(pointer.to_owned())
    );
    let close = Close {
        initiator: Initiator::Provider,
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
    decode_ws_case(valid().as_bytes(), &limits, &fixtures()).unwrap_err();
    let mut short = valid().replace("setup", "a").replace("ready", "b");
    short.push_str("execution_seconds: 3\n");
    decode_ws_case(short.as_bytes(), &limits, &fixtures()).unwrap();
    let binary_path = valid().replace("text: setup", "binary: ../outside.bin");
    decode_ws_case(
        binary_path.as_bytes(),
        &OracleLimits::default(),
        &fixtures(),
    )
    .unwrap_err();
}

#[test]
fn replays_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/websocket_oracle");
    let outcomes = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode_ws_case(&bytes, &OracleLimits::default(), &fixtures())
        })
        .collect::<Vec<_>>();
    assert!(!outcomes.is_empty());
}

async fn bounded_child() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start(GatewaySpec {
        command: Path::new("sleep").to_owned(),
        args: vec!["30".to_owned()],
        cwd: directory.path().to_owned(),
        env: Vec::new(),
        port: 0,
        log_path: directory.path().join("child.log"),
    })
    .unwrap();
    let process = Path::new("/proc").join(gateway.child.id().to_string());
    assert!(process.exists());
    timeout(Duration::from_millis(5), async move {
        let owned_gateway = gateway;
        pending::<()>().await;
        drop(owned_gateway);
    })
    .await
    .unwrap_err();
    assert!(!process.exists());
}

#[tokio::test]
async fn binary_payload() {
    let input = valid().replace("text: ready", "binary: provider.bin");
    let case = decode_ws_case(input.as_bytes(), &OracleLimits::default(), &fixtures()).unwrap();
    let provider = start_provider(case.clone(), limits(), settings(), &TokioClock)
        .await
        .unwrap();
    let mut observed = exchange_gateway(
        &format!("ws://{}", provider.address),
        &case,
        &bounds(),
        &TokioClock,
        &TokioTransport,
    )
    .await
    .unwrap();
    provider.finish(Duration::from_secs(2)).await.unwrap();
    compare_transcript(&case, &observed).unwrap();
    observed.frames[1].binary_data = Some(vec![0]);
    assert_eq!(
        compare_transcript(&case, &observed),
        Err("/provider/frames/0/binary".to_owned())
    );
}
