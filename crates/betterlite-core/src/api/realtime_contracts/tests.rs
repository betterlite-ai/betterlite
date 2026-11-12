#![cfg(test)]

use std::fs;
use std::path::Path;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde::Serialize;

use super::cache::RealtimeRequestBodyCache;
use super::close::{
    BackendClose, CloseFrame, ConnectionClosed, backend_close_from, client_close_code,
    close_after_handshake_refusal, realtime_error_event, upstream_handshake_close_code,
    websocket_close_reason,
};
use super::config::{
    BaseRealtimeConfig, BaseRealtimeHttpConfig, Environment, ErrorInfo, RealtimeUrl,
};
use super::decode::{
    RealtimeContract, RealtimeContractKind, RealtimeDecodeLimits, decode_realtime_contract,
    decode_realtime_frame,
};
use super::error::{HeaderMap, RealtimeContractError, WebSocketError};
use super::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use super::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};

const LIMITS: RealtimeDecodeLimits = RealtimeDecodeLimits { max_bytes: 1 << 20 };

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn encode<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn decode(kind: RealtimeContractKind, text: &str) -> RealtimeContract {
    decode_realtime_contract(text.as_bytes(), kind, LIMITS).unwrap()
}

#[test]
fn expiry_defaults_anchor_to_created_at() {
    assert_eq!(
        encode(&decode(RealtimeContractKind::Expiry, "{}")),
        parse(r#"{"anchor":"created_at","seconds":null}"#)
    );
    assert_eq!(
        encode(&decode(RealtimeContractKind::Expiry, r#"{"anchor":null}"#)),
        parse(r#"{"anchor":null,"seconds":null}"#)
    );
}

#[test]
fn empty_session_keeps_every_declared_field_null_and_unknown_fields() {
    assert_eq!(
        encode(&decode(RealtimeContractKind::Session, r#"{"future":1}"#)),
        parse(
            r#"{"type":null,"model":null,"instructions":null,"audio":null,"include":null,"max_output_tokens":null,"output_modalities":null,"tool_choice":null,"tools":null,"tracing":null,"truncation":null,"prompt":null,"future":1}"#
        )
    );
}

#[test]
fn session_round_trips_declared_values_and_unknown_objects() {
    let text = r#"{"type":"realtime","model":"m","instructions":"hi","audio":{"input":{"format":"pcm16"}},"include":["a"],"max_output_tokens":12,"output_modalities":["audio"],"tool_choice":"auto","tools":[{"type":"function"}],"tracing":"auto","truncation":{"type":"retention_ratio"},"prompt":{"id":"p"},"future":{"deep":true}}"#;
    assert_eq!(
        encode(&decode(RealtimeContractKind::Session, text)),
        parse(text)
    );
    let string_tokens = decode(
        RealtimeContractKind::Session,
        r#"{"max_output_tokens":"twelve"}"#,
    );
    assert_eq!(
        encode(&string_tokens),
        parse(
            r#"{"type":null,"model":null,"instructions":null,"audio":null,"include":null,"max_output_tokens":"twelve","output_modalities":null,"tool_choice":null,"tools":null,"tracing":null,"truncation":null,"prompt":null}"#
        )
    );
}

#[test]
fn client_secret_request_ignores_unknown_top_level_fields() {
    assert_eq!(
        encode(&decode(
            RealtimeContractKind::ClientSecretRequest,
            r#"{"model":"m","bogus":1,"session":{"model":"s","future":2}}"#,
        )),
        parse(
            r#"{"expires_after":null,"session":{"type":null,"model":"s","instructions":null,"audio":null,"include":null,"max_output_tokens":null,"output_modalities":null,"tool_choice":null,"tools":null,"tracing":null,"truncation":null,"prompt":null,"future":2},"model":"m"}"#
        )
    );
}

#[test]
fn client_secret_response_requires_a_value_and_accepts_an_empty_one() {
    assert!(matches!(
        decode_realtime_contract(
            br#"{"expires_at":1}"#,
            RealtimeContractKind::ClientSecretResponse,
            LIMITS
        ),
        Err(RealtimeContractError::Record(_))
    ));
    assert_eq!(
        encode(&decode(
            RealtimeContractKind::ClientSecretResponse,
            r#"{"value":""}"#
        )),
        parse(r#"{"expires_at":null,"value":"","session":null}"#)
    );
}

#[rstest]
#[case::top_level_wins(
    r#"{"model":"m","input_audio_transcription":{"model":"n"}}"#,
    Some("m")
)]
#[case::empty_top_level_falls_through(
    r#"{"model":"","input_audio_transcription":{"model":"n"}}"#,
    Some("n")
)]
#[case::nested_only(r#"{"input_audio_transcription":{"model":"n"}}"#, Some("n"))]
#[case::empty_nested_model(r#"{"input_audio_transcription":{"model":""}}"#, Some(""))]
#[case::neither("{}", None)]
#[case::empty_nested_object(r#"{"input_audio_transcription":{}}"#, None)]
#[case::missing_nested_model(r#"{"input_audio_transcription":{"language":"en"}}"#, None)]
fn transcription_resolved_model(#[case] text: &str, #[case] expected: Option<&str>) {
    let RealtimeContract::TranscriptionRequest(request) =
        decode(RealtimeContractKind::TranscriptionRequest, text)
    else {
        panic!("expected a transcription request");
    };
    assert_eq!(request.resolved_model(), expected);
}

#[test]
fn transcription_records_keep_unknown_fields() {
    let request =
        r#"{"model":"m","input_audio_transcription":{"model":"n","language":"en"},"future":1}"#;
    assert_eq!(
        encode(&decode(RealtimeContractKind::TranscriptionRequest, request)),
        parse(request)
    );
    let response = r#"{"client_secret":{"value":"v"},"future":2}"#;
    assert_eq!(
        encode(&decode(
            RealtimeContractKind::TranscriptionResponse,
            response
        )),
        parse(response)
    );
}

#[rstest]
#[case::tokens(
    r#"{"type":"tokens","input_tokens":3,"output_tokens":4,"total_tokens":7,"input_token_details":{"text_tokens":2,"audio_tokens":1}}"#
)]
#[case::duration(r#"{"type":"duration","seconds":1.5}"#)]
fn usage_variants_round_trip(#[case] text: &str) {
    let usage: AudioTranscriptionUsage = serde_json::from_str(text).unwrap();
    assert_eq!(encode(&usage), parse(text));
}

struct TestBackend;

#[async_trait]
impl RealtimeBackend for TestBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Ok(RealtimeMessage::Text(String::new()))
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

struct TestConfig;

#[async_trait]
impl BaseRealtimeConfig for TestConfig {
    type Error = RealtimeContractError;
    type Backend = TestBackend;

    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error> {
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Null,
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        })
    }
}

struct HttpClientConfig;

impl BaseRealtimeHttpConfig for HttpClientConfig {
    type Error = RealtimeContractError;

    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Self::Error> {
        Ok(api_base.unwrap_or_default().to_owned())
    }

    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Self::Error> {
        Ok(api_key.unwrap_or_default().to_owned())
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _api_version: Option<&str>,
    ) -> String {
        format!(
            "{}/v1/realtime/client_secrets",
            api_base.unwrap_or_default()
        )
    }

    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }
}

#[tokio::test]
async fn base_hooks_return_their_defaults() {
    let config = TestConfig;
    assert_eq!(config.session_configuration_request("m"), None);
    assert_eq!(config.session_close_usage("m"), None);
    assert_eq!(config.session_created_event("m", "session", None), None);
    assert!(
        config
            .open_backend("wss://backend", &HeaderMap::new())
            .await
            .unwrap()
            .is_none()
    );
    config.pace_backend_send(b"{}").await;
    assert!(!config.is_setup_message(&Json::Null));
    assert!(!config.is_content_message(&Json::Null));
    assert!(!config.requires_session_configuration());
}

#[rstest]
#[case::supplied(Some("https://api"), "https://api")]
#[case::absent(None, "")]
#[case::empty(Some(""), "")]
fn base_complete_url_returns_the_supplied_base_or_empty(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(TestConfig.get_complete_url(api_base, "m", None), expected);
}

#[test]
fn websocket_error_class_raises_and_http_error_class_returns() {
    let headers = HeaderMap::from([("retry-after".to_owned(), Json::Text("2".to_owned()))]);
    let error = ErrorInfo {
        message: "bad",
        status_code: 429,
        headers: &headers,
    };
    let raised = TestConfig.get_error_class(&error).unwrap_err();
    assert_eq!((raised.status_code, raised.message.as_str()), (429, "bad"));
    assert_eq!(raised.headers, headers);
    let returned = HttpClientConfig.get_error_class(&error);
    assert_eq!(
        (returned.status_code, returned.message.as_str()),
        (429, "bad")
    );
    assert_eq!(returned.headers, headers);
}

#[rstest]
#[case::none(None, "/v1/realtime/transcription_sessions", "/v1/realtime/calls")]
#[case::host(
    Some("https://host"),
    "https://host/v1/realtime/transcription_sessions",
    "https://host/v1/realtime/calls"
)]
#[case::v1_slash(
    Some("https://host/v1/"),
    "https://host/v1/realtime/transcription_sessions",
    "https://host/v1/v1/realtime/calls"
)]
fn realtime_http_url_defaults(
    #[case] api_base: Option<&str>,
    #[case] transcription: &str,
    #[case] calls: &str,
) {
    let url = RealtimeUrl {
        api_base,
        model: "m",
        api_version: Some("2025-04-01"),
    };
    assert_eq!(
        HttpClientConfig.get_transcription_session_url(&url),
        transcription
    );
    assert_eq!(HttpClientConfig.get_realtime_calls_url(&url), calls);
}

#[test]
fn realtime_calls_headers_use_only_the_ephemeral_bearer() {
    let key_headers = HttpClientConfig.get_realtime_calls_headers("ep-test");
    assert_eq!(key_headers.len(), 1);
    assert_eq!(
        key_headers.get("Authorization"),
        Some(&Json::Text("Bearer ep-test".to_owned()))
    );
    let empty_headers = HttpClientConfig.get_realtime_calls_headers("");
    assert_eq!(empty_headers.len(), 1);
    assert_eq!(
        empty_headers.get("Authorization"),
        Some(&Json::Text("Bearer ".to_owned()))
    );
}

#[test]
fn error_event_carries_only_type_error_and_message() {
    assert_eq!(
        parse(&realtime_error_event("bad", "server_error").unwrap()),
        parse(r#"{"type":"error","error":{"type":"server_error","message":"bad"}}"#)
    );
}

#[test]
fn websocket_close_reason_keeps_whole_characters() {
    assert_eq!(
        websocket_close_reason(&"a".repeat(122), "fb"),
        "a".repeat(122)
    );
    assert_eq!(
        websocket_close_reason(&"a".repeat(123), "fb"),
        "a".repeat(123)
    );
    assert_eq!(
        websocket_close_reason(&"a".repeat(124), "fallback"),
        "a".repeat(123)
    );
    assert_eq!(
        websocket_close_reason(&"\u{1f600}".repeat(31), "fallback"),
        "\u{1f600}".repeat(30)
    );
    assert_eq!(
        websocket_close_reason(&"\u{e9}".repeat(62), "fallback"),
        "\u{e9}".repeat(61)
    );
    assert_eq!(websocket_close_reason("", "fallback"), "fallback");
    let long_fallback = "f".repeat(130);
    assert_eq!(websocket_close_reason("", &long_fallback), long_fallback);
}

#[test]
fn client_close_codes_keep_external_codes_and_replace_the_rest() {
    let preserved = [
        1000_u16, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 3000, 4999,
    ]
    .map(client_close_code);
    assert_eq!(
        preserved,
        [
            1000_u16, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 3000, 4999
        ]
    );
    let replaced = [1005_u16, 1006, 1015, 2999, 5000, 0].map(client_close_code);
    assert_eq!(replaced, [1011_u16; 6]);
}

#[test]
fn upstream_handshake_close_codes() {
    assert_eq!(upstream_handshake_close_code(401), 1008);
    assert_eq!(upstream_handshake_close_code(403), 1008);
    assert_eq!(upstream_handshake_close_code(429), 1013);
    assert_eq!(upstream_handshake_close_code(500), 1011);
    assert_eq!(upstream_handshake_close_code(0), 1011);
}

#[test]
fn backend_close_messages() {
    let no_reason = BackendClose {
        code: 1006,
        reason: String::new(),
    };
    assert_eq!(
        no_reason.message(),
        "upstream websocket closed with code 1006"
    );
    let with_reason = BackendClose {
        code: 1006,
        reason: "gone".to_owned(),
    };
    assert_eq!(
        with_reason.message(),
        "upstream websocket closed with code 1006: gone"
    );
}

#[test]
fn backend_close_from_uses_the_received_frame_or_1006_with_the_error_text() {
    let received = ConnectionClosed {
        received: Some(CloseFrame {
            code: 1000,
            reason: "gone".to_owned(),
        }),
        message: "received 1000".to_owned(),
    };
    assert_eq!(
        backend_close_from(&received),
        BackendClose {
            code: 1000,
            reason: "gone".to_owned()
        }
    );
    let missing = ConnectionClosed {
        received: None,
        message: "sent 1000 (OK) bye".to_owned(),
    };
    let close = backend_close_from(&missing);
    assert_eq!(close.code, 1006);
    assert_eq!(close.reason, missing.to_string());
}

struct RecordingSocket {
    scope: WebSocketScope,
    send_fails: bool,
    close_fails: bool,
    calls: Vec<&'static str>,
    sent: Vec<String>,
    closed: Vec<(u16, Option<String>)>,
}

impl ScopedWebSocket for RecordingSocket {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for RecordingSocket {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.calls.push("send");
        if self.send_fails {
            return Err(WebSocketError {
                message: "send failed".to_owned(),
            });
        }
        self.sent.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Ok(String::new())
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        self.calls.push("close");
        if self.close_fails {
            return Err(WebSocketError {
                message: "close failed".to_owned(),
            });
        }
        self.closed.push((code, reason.map(str::to_owned)));
        Ok(())
    }
}

fn recording_socket() -> RecordingSocket {
    RecordingSocket {
        scope: WebSocketScope::default(),
        send_fails: false,
        close_fails: false,
        calls: Vec::new(),
        sent: Vec::new(),
        closed: Vec::new(),
    }
}

#[tokio::test]
async fn refusal_sends_the_error_event_before_closing_with_the_mapped_code() {
    let mut socket = recording_socket();
    close_after_handshake_refusal(&mut socket, 429)
        .await
        .unwrap();
    assert_eq!(socket.calls, ["send", "close"]);
    assert_eq!(
        parse(&socket.sent[0]),
        parse(
            r#"{"type":"error","error":{"type":"server_error","message":"Upstream realtime handshake rejected with HTTP 429"}}"#
        )
    );
    assert_eq!(
        socket.closed,
        vec![(
            1013,
            Some("Upstream realtime handshake rejected with HTTP 429".to_owned())
        )]
    );
}

#[tokio::test]
async fn refusal_attempts_the_close_after_a_failed_send() {
    let mut socket = recording_socket();
    socket.send_fails = true;
    close_after_handshake_refusal(&mut socket, 401)
        .await
        .unwrap();
    assert_eq!(socket.calls, ["send", "close"]);
    assert!(socket.sent.is_empty());
    assert_eq!(
        socket.closed,
        vec![(
            1008,
            Some("Upstream realtime handshake rejected with HTTP 401".to_owned())
        )]
    );
}

#[tokio::test]
async fn refusal_propagates_a_failed_close() {
    let mut socket = recording_socket();
    socket.close_fails = true;
    let error = close_after_handshake_refusal(&mut socket, 500)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the client websocket failed: close failed"
    );
    assert_eq!(socket.calls, ["send", "close"]);
}

#[rstest]
#[case::absent(None, "{\"model\": \"\"}")]
#[case::empty(Some(""), "{\"model\": \"\"}")]
#[case::plain(Some("m"), "{\"model\": \"m\"}")]
#[case::quote(Some("a\"b"), "{\"model\": \"a\"b\"}")]
#[case::newline(Some("a\nb"), "{\"model\": \"a\nb\"}")]
fn cached_body_interpolates_the_model_without_escaping(
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let cache = RealtimeRequestBodyCache::new();
    assert_eq!(cache.body(model), expected.as_bytes());
    assert_eq!(cache.occupancy().len, 1);
}

#[test]
fn cached_body_capacity_is_1000_and_eviction_keeps_the_bytes() {
    let cache = RealtimeRequestBodyCache::new();
    assert_eq!(cache.occupancy().capacity, 1000);
    (0..1001_usize).for_each(|index| drop(cache.body(Some(&format!("model-{index}")))));
    assert_eq!(cache.occupancy().len, 1000);
    assert_eq!(
        cache.body(Some("model-0")),
        b"{\"model\": \"model-0\"}".to_vec()
    );
}

#[rstest]
#[case::object("{}")]
#[case::array("[]")]
#[case::null("null")]
#[case::zero("0")]
#[case::big_number("123456789012345678901234567890")]
#[case::boolean("true")]
#[case::text("\"text\"")]
fn frame_decoder_accepts_every_json_value(#[case] payload: &str) {
    assert_eq!(
        encode(&decode_realtime_frame(payload.as_bytes(), LIMITS).unwrap()),
        parse(payload)
    );
}

#[rstest]
#[case::invalid_json(b"{bad")]
#[case::trailing_bytes(b"{} x")]
#[case::invalid_utf8(b"{\"a\":\"\xff\"}")]
fn frame_decoder_rejects_malformed_input(#[case] payload: &[u8]) {
    decode_realtime_frame(payload, LIMITS).unwrap_err();
}

#[test]
fn decoders_bound_the_input_size() {
    let limits = RealtimeDecodeLimits { max_bytes: 4_usize };
    decode_realtime_frame(b"null", limits).unwrap();
    assert!(matches!(
        decode_realtime_frame(b"false", limits),
        Err(RealtimeContractError::JsonTooLarge {
            observed: 5_usize,
            limit: 4_usize
        })
    ));
    assert!(matches!(
        decode_realtime_contract(b"{\"a\":1}", RealtimeContractKind::Session, limits),
        Err(RealtimeContractError::JsonTooLarge { .. })
    ));
}

#[test]
fn contract_decoder_names_the_field_that_does_not_match() {
    let error = decode_realtime_contract(
        br#"{"value":1}"#,
        RealtimeContractKind::ClientSecretResponse,
        LIMITS,
    )
    .unwrap_err();
    assert!(error.to_string().contains("value"), "{error}");
}

fn replay(bytes: &[u8]) {
    let (&kind, rest) = bytes.split_first().unwrap();
    match RealtimeContractKind::try_from(kind) {
        Ok(contract) => drop(decode_realtime_contract(rest, contract, LIMITS)),
        Err(_) => drop(decode_realtime_frame(rest, LIMITS)),
    }
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/realtime_contract");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| replay(&bytes))
        .count();
    assert!(replayed >= 10);
}
