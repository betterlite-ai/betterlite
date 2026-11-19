#![cfg(test)]

use betterlite_anthropic::api::messages_errors::frame::anthropic_error_sse_frame;
use betterlite_models::decisions::Json;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::super::call::StreamAbort;
use super::super::generator::DEFAULT_LIMITS;
use super::super::pings::with_sse_pings;
use super::super::sse::{Emitted, restamp_model_for};
use super::super::tail::{
    ANTHROPIC_PING_CHUNK, SSE_COMMENT_PING, SSE_STREAM_START_TAIL, advance_sse_tail,
    seal_open_sse_frame,
};
use super::{
    HookConfig, Rig, call, event_chunk, object, pending_call, sse_body, text_chunk, text_of, texts,
};
use crate::keepalive_interval::PingSetting;
use crate::stream_chunks::StreamItem;

#[test]
fn sse_uses_python_json() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("z", Json::Null),
        ("a", Json::Bool(true)),
        ("n", Json::Number(serde_json::Number::from(7_i64))),
        ("t", Json::Text(String::from("é"))),
    ]);
    let frames = texts(sse_body(&rig.generator, call(vec![Ok(item)]), None));
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"z\": null, \"a\": true, \"n\": 7, \"t\": \"\\u00e9\"}\n\n"
        )]
    );
}

#[test]
fn sse_passes_bytes_and_text() {
    let rig = Rig::new(HookConfig::plain());
    let items = vec![
        Ok(StreamItem::Bytes(Bytes::from_static(b"event: raw\n\n"))),
        Ok(text_chunk("plain text")),
    ];
    let frames = texts(sse_body(&rig.generator, call(items), None));
    assert_eq!(
        frames,
        vec![String::from("event: raw\n\n"), String::from("plain text")]
    );
}

#[test]
fn sse_restamps_message_start() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("type", Json::Text(String::from("message_start"))),
        (
            "message",
            Json::Object(object(&[
                ("id", Json::Text(String::from("msg"))),
                ("model", Json::Text(String::from("provider"))),
            ])),
        ),
    ]);
    let frames = texts(sse_body(
        &rig.generator,
        call(vec![Ok(item)]),
        Some(String::from("client-model")),
    ));
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"type\": \"message_start\", \"message\": {\"id\": \"msg\", \"model\": \"client-model\"}}\n\n"
        )]
    );
}

#[test]
fn sse_no_restamp_without_model() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("type", Json::Text(String::from("message_start"))),
        (
            "message",
            Json::Object(object(&[("model", Json::Text(String::from("provider")))])),
        ),
    ]);
    let frames = texts(sse_body(&rig.generator, call(vec![Ok(item)]), None));
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"type\": \"message_start\", \"message\": {\"model\": \"provider\"}}\n\n"
        )]
    );
}

#[test]
fn sse_empty_model_no_restamp() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("type", Json::Text(String::from("message_start"))),
        (
            "message",
            Json::Object(object(&[("model", Json::Text(String::from("provider")))])),
        ),
    ]);
    let frames = texts(sse_body(
        &rig.generator,
        call(vec![Ok(item)]),
        Some(String::new()),
    ));
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"type\": \"message_start\", \"message\": {\"model\": \"provider\"}}\n\n"
        )]
    );
}

#[test]
fn sse_holds_torn_frame() {
    let rig = Rig::new(HookConfig::plain());
    let torn = StreamItem::Bytes(Bytes::from_static(b"data: {\"x\":"));
    let frames = texts(sse_body(
        &rig.generator,
        call(vec![Ok(torn)]),
        Some(String::from("client-model")),
    ));
    assert_eq!(frames, vec![String::new(), String::from("data: {\"x\":")]);
}

#[test]
fn sse_without_restamper_writes_torn() {
    let rig = Rig::new(HookConfig::plain());
    let torn = StreamItem::Bytes(Bytes::from_static(b"data: {\"x\":"));
    let frames = texts(sse_body(&rig.generator, call(vec![Ok(torn)]), None));
    assert_eq!(frames, vec![String::from("data: {\"x\":")]);
}

#[test]
fn emitted_error_into_bytes() {
    let frame = anthropic_error_sse_frame(400, "nope", DEFAULT_LIMITS).expect("frame");
    let body = frame.body.clone();
    let emitted = Emitted::AnthropicError(Box::new(frame));
    assert_eq!(emitted.into_bytes(), Bytes::from(body));
}

#[test]
fn tail_keeps_last_four_bytes() {
    assert_eq!(advance_sse_tail(b"abc", b"def"), b"cdef".to_vec());
    assert_eq!(advance_sse_tail(b"", b"0123456789"), b"6789".to_vec());
    assert_eq!(
        advance_sse_tail(b"\n\n", b"data: x\n\n"),
        b" x\n\n".to_vec()
    );
    assert_eq!(advance_sse_tail(b"ab", b"cd"), b"abcd".to_vec());
}

#[test]
fn empty_write_keeps_tail() {
    assert_eq!(advance_sse_tail(b"abcdefgh", b""), b"abcdefgh".to_vec());
    assert_eq!(
        advance_sse_tail(SSE_STREAM_START_TAIL, b""),
        SSE_STREAM_START_TAIL.to_vec()
    );
}

#[test]
fn seal_closed_frame_is_empty() {
    assert_eq!(seal_open_sse_frame(b"data: x\n\n"), String::new());
    assert_eq!(seal_open_sse_frame(b"x\r\n\r\n"), String::new());
    assert_eq!(seal_open_sse_frame(b"x\r\r"), String::new());
    assert_eq!(seal_open_sse_frame(SSE_STREAM_START_TAIL), String::new());
}

#[test]
fn seal_after_torn_frame() {
    assert_eq!(
        seal_open_sse_frame(b"ta: {"),
        format!("\n{ANTHROPIC_PING_CHUNK}")
    );
}

#[test]
fn seal_after_line_break() {
    assert_eq!(
        seal_open_sse_frame(b"data: {\n"),
        String::from(ANTHROPIC_PING_CHUNK)
    );
    assert_eq!(
        seal_open_sse_frame(b"data: {\r"),
        String::from(ANTHROPIC_PING_CHUNK)
    );
}

#[test]
fn raw_name_from_metadata() {
    let data = object(&[(
        "metadata",
        Json::Object(object(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(true),
        )])),
    )]);
    assert_eq!(restamp_model_for(&data, Some("client")), None);
}

#[test]
fn raw_name_from_litellm_meta() {
    let data = object(&[(
        "litellm_metadata",
        Json::Object(object(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(true),
        )])),
    )]);
    assert_eq!(restamp_model_for(&data, Some("client")), None);
}

#[test]
fn raw_name_false_keeps_model() {
    let data = object(&[(
        "metadata",
        Json::Object(object(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(false),
        )])),
    )]);
    assert_eq!(
        restamp_model_for(&data, Some("client")),
        Some(String::from("client"))
    );
}

#[test]
fn raw_name_ignores_other_types() {
    let data = object(&[(
        "metadata",
        Json::Object(object(&[(
            "_complexity_router_return_raw_model_name",
            Json::Text(String::from("true")),
        )])),
    )]);
    assert_eq!(
        restamp_model_for(&data, Some("client")),
        Some(String::from("client"))
    );
    let flat = object(&[("metadata", Json::Text(String::from("meta")))]);
    assert_eq!(
        restamp_model_for(&flat, Some("client")),
        Some(String::from("client"))
    );
}

#[test]
fn restamp_none_without_client() {
    let data = object(&[]);
    assert_eq!(restamp_model_for(&data, None), None);
}

#[tokio::test(start_paused = true)]
async fn pings_fill_idle_gaps() {
    let inner: BoxStream<'static, Result<Emitted, StreamAbort>> = stream::once(async {
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        Ok::<Emitted, StreamAbort>(Emitted::Frame(Bytes::from_static(b"late")))
    })
    .boxed();
    let mut paced = with_sse_pings(
        inner,
        Some(&PingSetting::Seconds(1.0)),
        ANTHROPIC_PING_CHUNK,
    );
    let first = paced.next().await.expect("ping").expect("ok");
    assert_eq!(text_of(first), ANTHROPIC_PING_CHUNK);
    let second = paced.next().await.expect("ping").expect("ok");
    assert_eq!(text_of(second), ANTHROPIC_PING_CHUNK);
    let third = paced.next().await.expect("item").expect("ok");
    assert_eq!(text_of(third), "late");
    assert!(paced.next().await.is_none());
}

#[tokio::test]
async fn no_interval_means_no_pings() {
    let inner: BoxStream<'static, Result<Emitted, StreamAbort>> =
        stream::iter([Ok::<Emitted, StreamAbort>(Emitted::Frame(
            Bytes::from_static(b"only"),
        ))])
        .boxed();
    let mut plain = with_sse_pings(inner, None, SSE_COMMENT_PING);
    let first = plain.next().await.expect("item").expect("ok");
    assert_eq!(text_of(first), "only");
    assert!(plain.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn messages_stream_pings_the_gap() {
    let rig = Rig::new(HookConfig::plain());
    let mut body = rig.generator.messages_stream(pending_call(), None);
    let first = body.next().await.expect("ping").expect("ok");
    assert_eq!(text_of(first), ANTHROPIC_PING_CHUNK);
}

#[test]
fn messages_stream_restamps_the_client_model() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("type", Json::Text(String::from("message_start"))),
        (
            "message",
            Json::Object(object(&[("model", Json::Text(String::from("provider")))])),
        ),
    ]);
    let frames = texts(
        rig.generator
            .messages_stream(call(vec![Ok(item)]), Some("client-model")),
    );
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"type\": \"message_start\", \"message\": {\"model\": \"client-model\"}}\n\n"
        )]
    );
}

#[test]
fn messages_stream_keeps_the_raw_model() {
    let rig = Rig::new(HookConfig::plain());
    let item = event_chunk(&[
        ("type", Json::Text(String::from("message_start"))),
        (
            "message",
            Json::Object(object(&[("model", Json::Text(String::from("provider")))])),
        ),
    ]);
    let mut shared_call = call(vec![Ok(item)]);
    shared_call.request_data = object(&[(
        "metadata",
        Json::Object(object(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(true),
        )])),
    )]);
    let frames = texts(
        rig.generator
            .messages_stream(shared_call, Some("client-model")),
    );
    assert_eq!(
        frames,
        vec![String::from(
            "data: {\"type\": \"message_start\", \"message\": {\"model\": \"provider\"}}\n\n"
        )]
    );
}
