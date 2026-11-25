#![cfg(test)]

use std::fs;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use axum::body::{Body, to_bytes};
use bytes::Bytes;
use futures_util::Stream;
use futures_util::stream;
use rstest::rstest;

use super::{
    PEEK_LIMIT, has_top_level_method, initialize_client_info, is_initialize_request,
    is_jsonrpc_response, lossy_utf8, peek_routing_body,
};

const FULL_INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":"#,
    r#"{"protocolVersion":"2025-06-18","capabilities":{},"#,
    r#""clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const NO_METHOD_INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const NO_CAPABILITIES: &str = concat!(
    r#"{"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const NO_CLIENT_INFO: &str = concat!(
    r#"{"method":"initialize","params":"#,
    r#"{"protocolVersion":"2025-06-18","capabilities":{}}}"#,
);
const NO_CLIENT_VERSION: &str = concat!(
    r#"{"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"claude-ai"}}}"#,
);
const NO_PROTOCOL_VERSION: &str = concat!(
    r#"{"method":"initialize","params":"#,
    r#"{"capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const WRONG_METHOD: &str = concat!(
    r#"{"method":"tools/call","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const NULL_METHOD: &str = concat!(
    r#"{"method":null,"params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0"}}}"#,
);
const CLIENT_INFO_EXTRAS: &str = concat!(
    r#"{"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0","#,
    r#""title":"Claude","description":"desktop client"}}}"#,
);

#[test]
fn the_peek_limit_is_litellms_4096_bytes() {
    assert_eq!(PEEK_LIMIT, 4096);
}

#[tokio::test]
async fn a_small_body_is_sniffed_and_replayed_whole() {
    let (replayed, sniffed) = peek_routing_body(Body::from(FULL_INITIALIZE)).await;
    assert_eq!(sniffed, Bytes::from_static(FULL_INITIALIZE.as_bytes()));
    let whole = to_bytes(replayed, 10_000).await.unwrap();
    assert_eq!(whole, Bytes::from_static(FULL_INITIALIZE.as_bytes()));
}

#[tokio::test]
async fn an_empty_body_is_sniffed_as_nothing() {
    let (replayed, sniffed) = peek_routing_body(Body::empty()).await;
    assert!(sniffed.is_empty());
    assert!(to_bytes(replayed, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn every_read_frame_is_replayed() {
    let frames = vec![
        Ok::<Bytes, io::Error>(Bytes::from_static(b"abc")),
        Ok(Bytes::from_static(b"def")),
    ];
    let (replayed, sniffed) = peek_routing_body(Body::from_stream(stream::iter(frames))).await;
    assert_eq!(sniffed, Bytes::from_static(b"abcdef"));
    assert_eq!(
        to_bytes(replayed, 10_000).await.unwrap(),
        Bytes::from_static(b"abcdef")
    );
}

#[tokio::test]
async fn the_sniff_stops_at_the_peek_limit() {
    let first = vec![b'a'; PEEK_LIMIT];
    let second = vec![b'b'; 10];
    let frames = vec![
        Ok::<Bytes, io::Error>(Bytes::from(first.clone())),
        Ok(Bytes::from(second.clone())),
    ];
    let (replayed, sniffed) = peek_routing_body(Body::from_stream(stream::iter(frames))).await;
    assert_eq!(sniffed.len(), PEEK_LIMIT);
    assert!(sniffed.iter().all(|byte| *byte == b'a'));
    let whole = to_bytes(replayed, PEEK_LIMIT.saturating_add(10))
        .await
        .unwrap();
    assert_eq!(whole.len(), PEEK_LIMIT.saturating_add(10));
    assert_eq!(whole, Bytes::from([first, second].concat()));
}

// counts its polls, so a peek that reads past the limit is visible before the replay is consumed
struct CountingStream {
    frames: Vec<Result<Bytes, io::Error>>,
    polls: Arc<AtomicUsize>,
}

impl Stream for CountingStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::SeqCst);
        if this.frames.is_empty() {
            return Poll::Ready(None);
        }
        Poll::Ready(Some(this.frames.remove(0)))
    }
}

#[tokio::test]
async fn the_peek_reads_no_frame_past_the_limit() {
    let polls = Arc::new(AtomicUsize::new(0));
    let frames = vec![
        Ok::<Bytes, io::Error>(Bytes::from(vec![b'a'; PEEK_LIMIT])),
        Ok(Bytes::from_static(b"tail")),
    ];
    let source = CountingStream {
        frames,
        polls: Arc::clone(&polls),
    };
    let (replayed, sniffed) = peek_routing_body(Body::from_stream(source)).await;
    assert_eq!(sniffed.len(), PEEK_LIMIT);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    let whole = to_bytes(replayed, PEEK_LIMIT.saturating_add(4))
        .await
        .unwrap();
    assert_eq!(whole.len(), PEEK_LIMIT.saturating_add(4));
}

#[tokio::test]
async fn a_frame_past_the_limit_is_sniffed_in_part_and_replayed_whole() {
    let first = vec![b'a'; PEEK_LIMIT.saturating_sub(4)];
    let second = vec![b'b'; 10];
    let frames = vec![
        Ok::<Bytes, io::Error>(Bytes::from(first.clone())),
        Ok(Bytes::from(second.clone())),
    ];
    let (replayed, sniffed) = peek_routing_body(Body::from_stream(stream::iter(frames))).await;
    let kept = first.len();
    assert_eq!(sniffed.len(), PEEK_LIMIT);
    assert!(
        sniffed
            .get(..kept)
            .is_some_and(|head| head.iter().all(|byte| *byte == b'a'))
    );
    assert!(
        sniffed
            .get(kept..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == b'b'))
    );
    let whole = to_bytes(replayed, PEEK_LIMIT.saturating_add(6))
        .await
        .unwrap();
    assert_eq!(whole, Bytes::from([first, second].concat()));
}

#[tokio::test]
async fn an_error_frame_reaches_the_caller() {
    let frames = vec![
        Ok::<Bytes, io::Error>(Bytes::from_static(b"abc")),
        Err(io::Error::other("boom")),
    ];
    let (replayed, sniffed) = peek_routing_body(Body::from_stream(stream::iter(frames))).await;
    assert_eq!(sniffed, Bytes::from_static(b"abc"));
    let _error = to_bytes(replayed, 10_000).await.unwrap_err();
}

#[rstest]
#[case::ascii(b"hello", "hello")]
#[case::valid_multibyte("\u{e9}".as_bytes(), "\u{e9}")]
#[case::one_invalid_byte(b"a\xffb", "a\u{fffd}b")]
#[case::an_invalid_sequence_is_one_replacement(b"\xe2\x28\xa1", "\u{fffd}(\u{fffd}")]
#[case::a_truncated_sequence_is_one_replacement(b"ab\xe2\x82", "ab\u{fffd}")]
fn lossy_text_matches_pythons_decoding(#[case] bytes: &[u8], #[case] text: &str) {
    assert_eq!(lossy_utf8(bytes), text);
}

#[rstest]
#[case::empty("", false)]
#[case::initialize(r#"{"method": "initialize"}"#, true)]
#[case::notification(r#"{"method": "tools/call"}"#, false)]
#[case::no_method(r#"{"jsonrpc": "2.0"}"#, false)]
#[case::method_not_a_string(r#"{"method": 1}"#, false)]
#[case::null_method(r#"{"method": null}"#, false)]
#[case::broken_json(r#"{"method":"#, false)]
#[case::a_json_array("[]", false)]
#[case::a_json_string(r#""initialize""#, false)]
#[case::nested_method(r#"{"result": {"method": "initialize"}}"#, false)]
#[case::initialize_with_extras(FULL_INITIALIZE, true)]
fn an_initialize_request_is_detected(#[case] body: &str, #[case] expected: bool) {
    assert_eq!(is_initialize_request(body.as_bytes()), expected);
}

#[rstest]
#[case::full(FULL_INITIALIZE, Some(("claude-ai", "0.1.0")))]
#[case::method_defaults_to_initialize(NO_METHOD_INITIALIZE, Some(("claude-ai", "0.1.0")))]
#[case::without_capabilities(NO_CAPABILITIES, None)]
#[case::without_client_info(NO_CLIENT_INFO, None)]
#[case::without_client_version(NO_CLIENT_VERSION, None)]
#[case::without_protocol_version(NO_PROTOCOL_VERSION, None)]
#[case::wrong_method(WRONG_METHOD, None)]
#[case::null_method(NULL_METHOD, None)]
#[case::client_info_extras(CLIENT_INFO_EXTRAS, Some(("claude-ai", "0.1.0")))]
#[case::broken_json("{", None)]
#[case::empty("", None)]
#[case::not_an_object("[]", None)]
fn initialize_client_info_validates_the_sdk_request(
    #[case] body: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let found = initialize_client_info(body.as_bytes());
    let summary = found.map(|info| (info.name, info.version));
    assert_eq!(
        summary,
        expected.map(|(name, version)| (name.to_owned(), version.to_owned()))
    );
}

#[rstest]
#[case::root_key(r#"{"method":"initialize"}"#, true)]
#[case::key_before_the_value(r#"{"method":"initialize","params":{}}"#, true)]
#[case::nested_method(r#"{"result":{"method":"tools/call"}}"#, false)]
#[case::method_as_a_value(r#"{"a":"method"}"#, false)]
#[case::method_in_an_array(r#"["method"]"#, false)]
#[case::method_in_an_array_object(r#"{"a":[{"method":"x"}]}"#, false)]
#[case::other_case(r#"{"Method":"x"}"#, false)]
#[case::bare_key_before_a_value(r#"{a:"method"}"#, false)]
#[case::truncated_after_the_key(r#"{"method":"initial"#, true)]
#[case::truncated_inside_the_key(r#"{"meth"#, false)]
#[case::key_never_closes(r#"{"method"#, false)]
#[case::escaped_quote_before(r#"{"x\"y":1,"method":2}"#, true)]
#[case::escaped_quote_in_a_value(r#"{"a":"x\"y","method":1}"#, true)]
#[case::string_value_before_the_key(r#"{"a":"x","method":1}"#, true)]
#[case::after_the_root_closes(r#"{"a":1}{"method":1}"#, false)]
#[case::root_object_then_method(r#"{"a":{"b":1},"method":"x"}"#, true)]
#[case::empty("", false)]
fn a_top_level_method_key_is_detected(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(has_top_level_method(text), expected);
}

#[rstest]
#[case::result(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, true)]
#[case::error(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1}}"#, true)]
#[case::null_id(r#"{"jsonrpc":"2.0","id":null,"result":{}}"#, true)]
#[case::method_present(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","result":{}}"#, false)]
#[case::no_id(r#"{"jsonrpc":"2.0","result":{}}"#, false)]
#[case::wrong_version(r#"{"jsonrpc":"1.0","id":1,"result":{}}"#, false)]
#[case::numeric_version(r#"{"jsonrpc":2.0,"id":1,"result":{}}"#, false)]
#[case::no_result_or_error(r#"{"jsonrpc":"2.0","id":1}"#, false)]
#[case::a_json_array(r#"[{"jsonrpc":"2.0","id":1,"result":{}}]"#, false)]
#[case::truncated_result(concat!(r#"{"jsonrpc": "2.0", "id": 1, "#, r#""result": {"x": "y""#), true)]
#[case::truncated_request_with_a_result_param(
    concat!(
        r#"{"jsonrpc":"2.0","method":"tools/call","#,
        r#""params":{"result":"#
    ),
    false
)]
#[case::truncated_result_holding_a_method(r#"{"jsonrpc":"2.0","id":1,"result":{"method":"#, true)]
#[case::text_without_result(r#"{"jsonrpc": "2.0", "id": 1, "message": "x"#, false)]
#[case::empty("", false)]
fn a_jsonrpc_response_is_detected(#[case] body: &str, #[case] expected: bool) {
    assert_eq!(is_jsonrpc_response(body.as_bytes()), expected);
}

#[rstest]
fn replays_the_mcp_routing_corpus(#[files("fuzz/corpus/mcp_routing_peek/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _initialize = is_initialize_request(&bytes);
    let _client = initialize_client_info(&bytes);
    let _response = is_jsonrpc_response(&bytes);
    let _method = has_top_level_method(&lossy_utf8(&bytes));
}
