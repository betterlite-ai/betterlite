use betterlite_core::api::realtime_contracts::close::CloseFrame;
use serde_json::{Map, Value};

use crate::passthrough::{CLOSE_REASON_MAX_BYTES, SENDABLE_CLOSE_CODES};

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected an object, got {other}"),
    }
}

fn frame(code: u16, reason: &str) -> CloseFrame {
    CloseFrame {
        code,
        reason: reason.to_owned(),
    }
}

fn rewriter(model: &str) -> String {
    format!("projects/p/locations/l/models/{model}")
}

#[test]
fn sendable_close_codes_are_the_wire_codes() {
    assert_eq!(
        SENDABLE_CLOSE_CODES,
        [
            1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014
        ]
    );
    assert!(!SENDABLE_CLOSE_CODES.contains(&1004));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1005));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1006));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1015));
}

#[test]
fn the_close_reason_budget_is_123_bytes() {
    assert_eq!(CLOSE_REASON_MAX_BYTES, 123);
}

mod passthrough_rewrite_setup_model {
    use rstest::rstest;
    use serde_json::json;

    use super::rewriter;
    use crate::passthrough::websocket::rewrite_live_setup_model;

    #[rstest]
    #[case::empty("")]
    #[case::plain_text("not json")]
    #[case::array("[1,2]")]
    #[case::object_without_setup(r#"{"model":"m"}"#)]
    #[case::setup_not_an_object(r#"{"setup":"m"}"#)]
    #[case::setup_without_a_model(r#"{"setup":{"temperature":1}}"#)]
    #[case::setup_model_not_a_string(r#"{"setup":{"model":3}}"#)]
    fn frames_that_are_not_setup_frames_stay_identical(#[case] text: &str) {
        assert_eq!(rewrite_live_setup_model(text, None), text);
        assert_eq!(rewrite_live_setup_model(text, Some(&rewriter)), text);
    }

    #[test]
    fn an_unchanged_rewrite_returns_the_input_bytes() {
        let same = |model: &str| model.to_owned();
        let input = r#"{ "setup" : { "model" : "alias" } }"#;
        assert_eq!(rewrite_live_setup_model(input, Some(&same)), input);
    }

    #[test]
    fn a_setup_frame_gets_the_rewritten_model() {
        let input = r#"{"setup":{"model":"alias","generationConfig":{"n":1}},"extra":true}"#;
        let output = rewrite_live_setup_model(input, Some(&rewriter));
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value,
            json!({
                "setup": {
                    "model": "projects/p/locations/l/models/alias",
                    "generationConfig": {"n": 1}
                },
                "extra": true
            })
        );
    }
}

mod passthrough_resolved_setup {
    use serde_json::json;

    use super::{object, rewriter};
    use crate::passthrough::websocket::resolved_vertex_live_setup;

    #[test]
    fn without_a_rewriter_the_setup_is_returned_unchanged() {
        let setup = object(json!({"model": "alias", "temperature": 1}));
        assert_eq!(resolved_vertex_live_setup(&setup, None), setup);
    }

    #[test]
    fn the_rewritten_model_replaces_the_alias() {
        let setup = object(json!({"model": "alias", "temperature": 1}));
        assert_eq!(
            resolved_vertex_live_setup(&setup, Some(&rewriter)),
            object(json!({
                "model": "projects/p/locations/l/models/alias",
                "temperature": 1
            }))
        );
    }

    #[test]
    fn a_setup_without_a_string_model_is_returned_unchanged() {
        let setup = object(json!({"model": 3, "temperature": 1}));
        assert_eq!(resolved_vertex_live_setup(&setup, Some(&rewriter)), setup);
    }

    #[test]
    fn a_setup_without_a_model_is_returned_unchanged() {
        let setup = object(json!({"temperature": 1}));
        assert_eq!(resolved_vertex_live_setup(&setup, Some(&rewriter)), setup);
    }
}

mod passthrough_json_object_frame {
    use rstest::rstest;
    use serde_json::Value;
    use serde_json::json;

    use crate::passthrough::websocket::json_object_frame;

    #[test]
    fn an_object_frame_is_decoded() {
        assert_eq!(
            json_object_frame(br#"{"setup":{"model":"m"},"stream":true}"#).map(Value::Object),
            Some(json!({"setup": {"model": "m"}, "stream": true}))
        );
    }

    #[rstest]
    #[case::array(b"[1,2]")]
    #[case::string(b"\"text\"")]
    #[case::number(b"3")]
    #[case::null(b"null")]
    #[case::unterminated(b"{")]
    #[case::empty(b"")]
    #[case::invalid_utf8(b"{\"a\":\"\xff\"}")]
    fn frames_that_are_not_objects_are_rejected(#[case] frame: &[u8]) {
        assert_eq!(json_object_frame(frame), None);
    }
}

mod passthrough_truncated_close_reason {
    use rstest::rstest;

    use crate::passthrough::websocket::truncated_close_reason;

    #[rstest]
    #[case::empty(String::new(), String::new())]
    #[case::short("boom".to_owned(), "boom".to_owned())]
    #[case::exactly_the_limit("a".repeat(123), "a".repeat(123))]
    #[case::one_byte_over("a".repeat(124), "a".repeat(123))]
    #[case::fits_a_two_byte_char("a".repeat(121) + "\u{e9}", "a".repeat(121) + "\u{e9}")]
    #[case::splits_a_two_byte_char("a".repeat(122) + "\u{e9}", "a".repeat(122))]
    #[case::splits_a_four_byte_char("a".repeat(122) + "\u{1f600}", "a".repeat(122))]
    #[case::ends_on_a_boundary("a".repeat(123) + "\u{e9}", "a".repeat(123))]
    fn truncates_without_splitting_a_character(#[case] reason: String, #[case] expected: String) {
        assert_eq!(truncated_close_reason(&reason), expected);
    }
}

mod passthrough_client_socket_open {
    use rstest::rstest;

    use crate::passthrough::websocket::{WebSocket, WebSocketState, client_socket_is_open};

    #[rstest]
    #[case::both_connecting(WebSocketState::Connecting, WebSocketState::Connecting, true)]
    #[case::client_connecting(WebSocketState::Connecting, WebSocketState::Connected, true)]
    #[case::application_connecting(WebSocketState::Connected, WebSocketState::Connecting, true)]
    #[case::both_connected(WebSocketState::Connected, WebSocketState::Connected, true)]
    #[case::client_disconnected(WebSocketState::Disconnected, WebSocketState::Connected, false)]
    #[case::application_disconnected(
        WebSocketState::Connected,
        WebSocketState::Disconnected,
        false
    )]
    #[case::both_disconnected(WebSocketState::Disconnected, WebSocketState::Disconnected, false)]
    #[case::client_disconnected_first(
        WebSocketState::Disconnected,
        WebSocketState::Connecting,
        false
    )]
    #[case::application_disconnected_first(
        WebSocketState::Connecting,
        WebSocketState::Disconnected,
        false
    )]
    fn needs_both_halves_live(
        #[case] client_state: WebSocketState,
        #[case] application_state: WebSocketState,
        #[case] expected: bool,
    ) {
        let socket = WebSocket {
            client_state,
            application_state,
        };
        assert_eq!(client_socket_is_open(socket), expected);
    }
}

mod passthrough_upstream_close {
    use betterlite_core::api::realtime_contracts::close::CloseFrame;
    use rstest::rstest;

    use super::frame;
    use crate::passthrough::websocket::upstream_close_to_relay;

    #[rstest]
    #[case::no_close(None, None)]
    #[case::normal_without_a_reason(frame(1000, ""), None)]
    #[case::normal_with_a_reason(frame(1000, "done"), Some(frame(1000, "done")))]
    #[case::going_away(frame(1001, ""), Some(frame(1001, "")))]
    #[case::protocol_error(frame(1002, "bad"), Some(frame(1002, "bad")))]
    #[case::unsupported_data(frame(1003, ""), Some(frame(1003, "")))]
    #[case::reserved(frame(1004, ""), None)]
    #[case::no_status_received(frame(1005, "x"), None)]
    #[case::abnormal_closure(frame(1006, ""), None)]
    #[case::invalid_data(frame(1007, ""), Some(frame(1007, "")))]
    #[case::policy_violation(frame(1008, ""), Some(frame(1008, "")))]
    #[case::message_too_big(frame(1009, ""), Some(frame(1009, "")))]
    #[case::mandatory_extension(frame(1010, ""), Some(frame(1010, "")))]
    #[case::internal_error(frame(1011, ""), Some(frame(1011, "")))]
    #[case::service_restart(frame(1012, ""), Some(frame(1012, "")))]
    #[case::try_again_later(frame(1013, ""), Some(frame(1013, "")))]
    #[case::bad_gateway(frame(1014, ""), Some(frame(1014, "")))]
    #[case::tls_handshake(frame(1015, ""), None)]
    #[case::private_start(frame(3000, ""), Some(frame(3000, "")))]
    #[case::private_end(frame(4999, "private"), Some(frame(4999, "private")))]
    #[case::past_the_private_range(frame(5000, ""), None)]
    #[case::below_the_private_range(frame(2999, ""), None)]
    #[case::zero(frame(0, ""), None)]
    fn relays_only_sendable_closes(
        #[case] upstream: Option<CloseFrame>,
        #[case] expected: Option<CloseFrame>,
    ) {
        assert_eq!(upstream_close_to_relay(upstream), expected);
    }
}
