#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use rstest::rstest;

use super::super::types::SseDataGenerator;
use super::{
    RecordingHooks, body_of, chat_chunk, chunk_frame, collect, count, count_prefix, expected_chunk,
    fallback_hidden, frames, full_chunk, plain_chunk, plain_generator, recorded, recorded_outcomes,
    recording_generator, request, source, usage_artifact, with_hidden,
};
use crate::stream_chunks::{DONE_FRAME, MAX_RAW_SSE_BUFFER_CHARS, StreamItem};

fn include_request() -> BTreeMap<String, JsonValue> {
    request(&[("include_fallback_errors", JsonValue::from(true))])
}

fn fallback_generator() -> (SseDataGenerator<String>, Arc<RecordingHooks>) {
    let (mut generator, hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
        None,
    );
    generator.expose_fallback_errors = true;
    (generator, hooks)
}

#[test]
fn chat_frames_restamp_model() {
    let data = request(&[("model", JsonValue::from("requested-model"))]);
    let out = frames(body_of(
        &plain_generator(),
        source(vec![Ok(chat_chunk("hello"))], JsonObject::new()),
        data,
        false,
    ));
    assert_eq!(
        out,
        vec![
            chunk_frame(&expected_chunk("hello", Some("requested-model"))),
            DONE_FRAME.to_owned(),
        ]
    );
}

#[test]
fn empty_request_keeps_model() {
    let out = frames(body_of(
        &plain_generator(),
        source(vec![Ok(chat_chunk("hello"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(
        out,
        vec![
            chunk_frame(&expected_chunk("hello", None)),
            DONE_FRAME.to_owned(),
        ]
    );
}

#[rstest]
#[case::true_bool(JsonValue::from(true), 0)]
#[case::false_bool(JsonValue::from(false), 1)]
#[case::text(JsonValue::from("true"), 1)]
#[case::number(JsonValue::from(1_i64), 1)]
#[case::null(JsonValue::Null, 1)]
fn skip_done_reads_json_bool(#[case] value: JsonValue, #[case] done_frames: usize) {
    let data = request(&[("_litellm_skip_openai_stream_done", value)]);
    let out = frames(body_of(
        &plain_generator(),
        source(Vec::new(), JsonObject::new()),
        data,
        false,
    ));
    assert_eq!(out.len(), done_frames);
}

#[test]
fn wrapped_skips_deferred_logging() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    hooks.set_wrap(true);
    let out = frames(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], "data: wrapped\n\n");
    let log = recorded(&events);
    assert_eq!(count(&log, "wrap"), 1);
    assert_eq!(count(&log, "deferred"), 0);
}

#[test]
fn unwrapped_logs_once() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    let out = frames(body_of(
        &generator,
        source(
            vec![Ok(chat_chunk("x")), Ok(chat_chunk("y"))],
            JsonObject::new(),
        ),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 3);
    let log = recorded(&events);
    assert_eq!(count(&log, "wrap"), 0);
    assert_eq!(count(&log, "deferred"), 1);
}

#[test]
fn capability_flags_are_read_once() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    let out = frames(body_of(
        &generator,
        source(
            vec![Ok(chat_chunk("x")), Ok(chat_chunk("y"))],
            JsonObject::new(),
        ),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 3);
    let log = recorded(&events);
    assert_eq!(count(&log, "needs-wrap"), 1);
    assert_eq!(count(&log, "needs-chunk"), 1);
    assert_eq!(count_prefix(&log, "chunk:"), 0);
}

#[test]
fn chunk_hook_sees_str_so_far() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    hooks.set_per_chunk(true);
    let items = vec![
        Ok(chat_chunk("")),
        Ok(chat_chunk("one")),
        Ok(plain_chunk()),
        Ok(full_chunk("two")),
        Ok(chat_chunk("three")),
    ];
    let out = frames(body_of(
        &generator,
        source(items, JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 6);
    let log = recorded(&events);
    let seen: Vec<&str> = log
        .iter()
        .filter_map(|event| event.strip_prefix("chunk:"))
        .collect();
    assert_eq!(seen, ["<none>", "<none>", "one", "one", "onetwo"]);
}

#[test]
fn chunk_failure_emits_error() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    hooks.set_per_chunk(true);
    hooks.set_chunk_fails(true);
    let out = collect(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 1);
    let frame = String::from_utf8(out[0].as_ref().expect("frame").to_vec()).expect("utf8");
    assert_eq!(
        frame,
        "data: {\"error\":{\"message\":\"guardrail failed\",\"type\":\"None\",\"param\":\"None\",\"code\":\"500\"}}\n\n"
    );
    assert_eq!(count(&recorded(&events), "failure:guardrail failed"), 1);
}

#[test]
fn response_fallback_is_latched() {
    let (generator, _hooks) = fallback_generator();
    let hidden = fallback_hidden(Some("from-response"), &["response-error"]);
    let out = frames(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], hidden),
        include_request(),
        false,
    ));
    assert_eq!(out.len(), 3);
    assert!(out[0].contains("\"model\":\"from-response\""));
    assert!(out[1].contains("litellm-fallback-metadata"));
    assert!(out[1].contains("response-error"));
    assert!(out[1].contains("\"fallback_model\":\"from-response\""));
    assert_eq!(out[2], DONE_FRAME);
}

#[test]
fn chunk_fallback_is_latched() {
    let (generator, _hooks) = fallback_generator();
    let items = vec![
        Ok(with_hidden(
            chat_chunk("first"),
            fallback_hidden(Some("fallback-model"), &["first-error"]),
        )),
        Ok(with_hidden(
            chat_chunk("second"),
            fallback_hidden(Some("other-model"), &["second-error"]),
        )),
    ];
    let out = frames(body_of(
        &generator,
        source(items, JsonObject::new()),
        include_request(),
        false,
    ));
    assert_eq!(out.len(), 4);
    assert!(out[0].contains("\"model\":\"fallback-model\""));
    assert!(out[1].contains("litellm-fallback-metadata"));
    assert!(out[1].contains("first-error"));
    assert!(!out[1].contains("second-error"));
    assert!(out[2].contains("\"model\":\"fallback-model\""));
    assert_eq!(out[3], DONE_FRAME);
}

#[test]
fn fallback_errors_keep_first_list() {
    let (generator, _hooks) = fallback_generator();
    let hidden = fallback_hidden(None, &["response-error"]);
    let items = vec![Ok(with_hidden(
        chat_chunk("x"),
        fallback_hidden(Some("chunk-model"), &["chunk-error"]),
    ))];
    let out = frames(body_of(
        &generator,
        source(items, hidden),
        include_request(),
        false,
    ));
    assert_eq!(out.len(), 3);
    assert!(out[0].contains("\"model\":\"chunk-model\""));
    assert!(out[1].contains("\"fallback_model\":\"chunk-model\""));
    assert!(out[1].contains("response-error"));
    assert!(!out[1].contains("chunk-error"));
}

#[derive(Clone, Copy)]
enum Gate {
    Open,
    ExposedOff,
    RequestFlagOff,
}

#[rstest]
#[case::open(Gate::Open)]
#[case::exposed_off(Gate::ExposedOff)]
#[case::request_flag_off(Gate::RequestFlagOff)]
fn fallback_event_needs_both_gates(#[case] gate: Gate) {
    let (mut generator, _hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
        None,
    );
    let (expose, data) = match gate {
        Gate::Open => (true, include_request()),
        Gate::ExposedOff => (false, include_request()),
        Gate::RequestFlagOff => (true, request(&[])),
    };
    generator.expose_fallback_errors = expose;
    let hidden = fallback_hidden(Some("fallback-model"), &["first-error"]);
    let out = frames(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], hidden),
        data,
        false,
    ));
    let open = matches!(gate, Gate::Open);
    assert_eq!(out.len(), if open { 3 } else { 2 });
    assert!(out[0].contains("\"model\":\"fallback-model\""));
    let event_written = out
        .iter()
        .any(|frame| frame.contains("litellm-fallback-metadata"));
    assert_eq!(event_written, open);
    assert_eq!(out.last().expect("done"), DONE_FRAME);
}

#[rstest]
#[case::strip_on(JsonValue::from(true), 1)]
#[case::strip_off(JsonValue::from(false), 2)]
#[case::strip_null(JsonValue::Null, 2)]
#[case::strip_text(JsonValue::from("true"), 2)]
fn strips_only_when_enabled(#[case] value: JsonValue, #[case] written: usize) {
    let data = request(&[("_litellm_strip_stream_usage", value)]);
    let out = frames(body_of(
        &plain_generator(),
        source(vec![Ok(usage_artifact())], JsonObject::new()),
        data,
        false,
    ));
    assert_eq!(out.len(), written);
    assert_eq!(out[written - 1], DONE_FRAME);
}

#[test]
fn usage_skip_emits_fallback_once() {
    let (generator, _hooks) = fallback_generator();
    let data = request(&[
        ("include_fallback_errors", JsonValue::from(true)),
        ("_litellm_strip_stream_usage", JsonValue::from(true)),
    ]);
    let hidden = fallback_hidden(Some("fallback-model"), &["first-error"]);
    let items = vec![Ok(usage_artifact()), Ok(chat_chunk("after"))];
    let out = frames(body_of(&generator, source(items, hidden), data, false));
    assert_eq!(out.len(), 3);
    assert!(out[0].contains("litellm-fallback-metadata"));
    assert!(out[1].contains("\"model\":\"fallback-model\""));
    assert_eq!(out[2], DONE_FRAME);
}

#[test]
fn raw_frames_keep_delimiters() {
    let data = request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]);
    let items = vec![
        Ok(StreamItem::Bytes(Bytes::from_static(
            b"data: one\n\ndata: tw",
        ))),
        Ok(StreamItem::Text(String::from("o"))),
    ];
    let out = frames(body_of(
        &plain_generator(),
        source(items, JsonObject::new()),
        data,
        false,
    ));
    assert_eq!(
        out,
        vec![
            String::from("data: one\n\n"),
            String::from("data: two\n\n"),
            DONE_FRAME.to_owned(),
        ]
    );
}

#[test]
fn raw_flushes_partial_frame() {
    let data = request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]);
    let out = frames(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Bytes(Bytes::from_static(b"data: one")))],
            JsonObject::new(),
        ),
        data,
        false,
    ));
    assert_eq!(
        out,
        vec![String::from("data: one\n\n"), DONE_FRAME.to_owned()]
    );
}

#[test]
fn raw_keeps_complete_frame() {
    let data = request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]);
    let out = frames(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Bytes(Bytes::from_static(
                b"data: two\r\n\r\n",
            )))],
            JsonObject::new(),
        ),
        data,
        false,
    ));
    assert_eq!(
        out,
        vec![String::from("data: two\r\n\r\n"), DONE_FRAME.to_owned()]
    );
}

#[test]
fn raw_rejects_overflow() {
    let data = request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]);
    let mut payload = String::from("data: complete\n\n");
    payload.push_str(&"x".repeat(MAX_RAW_SSE_BUFFER_CHARS + 1));
    let out = collect(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Bytes(Bytes::from(payload)))],
            JsonObject::new(),
        ),
        data,
        false,
    ));
    assert_eq!(out.len(), 2);
    assert_eq!(
        String::from_utf8(out[0].as_ref().expect("frame").to_vec()).expect("utf8"),
        "data: complete\n\n"
    );
    let error = String::from_utf8(out[1].as_ref().expect("frame").to_vec()).expect("utf8");
    assert!(
        error.contains("Raw SSE stream exceeded maximum buffered size without a frame delimiter")
    );
    assert!(error.contains("\"code\":\"500\""));
}

#[test]
fn raw_accepts_limit_characters() {
    let data = request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]);
    let mut payload = String::from("data: complete\n\n");
    payload.push_str(&"é".repeat(MAX_RAW_SSE_BUFFER_CHARS));
    let out = frames(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Bytes(Bytes::from(payload)))],
            JsonObject::new(),
        ),
        data,
        false,
    ));
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], "data: complete\n\n");
    assert_eq!(out[1].chars().count(), MAX_RAW_SSE_BUFFER_CHARS + 2);
    assert!(out[1].ends_with("\n\n"));
    assert_eq!(out[2], DONE_FRAME);
}

#[rstest]
#[case::data(b"data: payload", "data: payload\n\n")]
#[case::event(b"event: update", "event: update\n\n")]
#[case::comment(b": keepalive", ": keepalive\n\n")]
#[case::delimited(b"data: payload\n\n", "data: payload\n\n")]
fn non_raw_bytes_relay(#[case] raw: &[u8], #[case] expected: &str) {
    let out = frames(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Bytes(Bytes::copy_from_slice(raw)))],
            JsonObject::new(),
        ),
        request(&[]),
        false,
    ));
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], expected);
    assert_eq!(out[1], DONE_FRAME);
}

#[test]
fn text_error_stops_loop() {
    let items = vec![
        Ok(StreamItem::Text(String::from("data: failed"))),
        Ok(chat_chunk("ignored")),
    ];
    let out = frames(body_of(
        &plain_generator(),
        source(items, JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(
        out,
        vec![String::from("data: failed"), DONE_FRAME.to_owned()]
    );
}

#[test]
fn text_error_skips_fallback() {
    let (generator, _hooks) = fallback_generator();
    let hidden = fallback_hidden(Some("fallback-model"), &["first-error"]);
    let items = vec![Ok(StreamItem::Text(String::from("data: failed")))];
    let out = frames(body_of(
        &generator,
        source(items, hidden),
        include_request(),
        false,
    ));
    assert_eq!(
        out,
        vec![String::from("data: failed"), DONE_FRAME.to_owned()]
    );
}

#[rstest]
#[case::plain("hello", "data: hello\n\n")]
#[case::no_space("data:hello", "data: data:hello\n\n")]
#[case::event("event: update", "data: event: update\n\n")]
#[case::comment(": ping", "data: : ping\n\n")]
fn plain_text_becomes_data_frame(#[case] text: &str, #[case] expected: &str) {
    let out = frames(body_of(
        &plain_generator(),
        source(
            vec![Ok(StreamItem::Text(text.to_owned()))],
            JsonObject::new(),
        ),
        request(&[]),
        false,
    ));
    assert_eq!(out, vec![expected.to_owned(), DONE_FRAME.to_owned()]);
}

#[test]
fn bytes_replace_invalid_utf8() {
    let items = vec![
        Ok(StreamItem::Bytes(Bytes::from_static(b"a\xffb"))),
        Ok(StreamItem::Bytes(Bytes::from_static(b"\xe2"))),
        Ok(StreamItem::Bytes(Bytes::from_static(b"\x82\xac"))),
    ];
    let out = frames(body_of(
        &plain_generator(),
        source(items, JsonObject::new()),
        request(&[]),
        false,
    ));
    assert_eq!(out[0], "data: a\u{fffd}b\n\n");
    assert_eq!(out[1], "data: \u{fffd}\n\n");
    assert_eq!(out[2], "data: \u{fffd}\u{fffd}\n\n");
    assert_eq!(out[3], DONE_FRAME);
}

#[test]
fn generate_is_lazy() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) = recording_generator(Arc::clone(&events), Arc::clone(&outcomes), None);
    let body = body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[]),
        true,
    );
    assert!(recorded(&events).is_empty());
    assert!(recorded_outcomes(&outcomes).is_empty());
    drop(body);
    assert!(recorded(&events).is_empty());
    assert!(recorded_outcomes(&outcomes).is_empty());
}

#[tokio::test]
async fn overflow_yields_before_failure() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    let mut text = String::from("data: complete\n\n");
    text.push_str(&"x".repeat(MAX_RAW_SSE_BUFFER_CHARS + 1));
    let mut body = body_of(
        &generator,
        source(vec![Ok(StreamItem::Text(text))], JsonObject::new()),
        request(&[("_litellm_raw_sse_stream", JsonValue::from(true))]),
        false,
    );
    assert_eq!(
        body.next().await.expect("frame").expect("bytes"),
        "data: complete\n\n"
    );
    assert_eq!(count_prefix(&recorded(&events), "failure:"), 0);
    assert!(body.next().await.expect("error frame").is_ok());
    assert_eq!(count_prefix(&recorded(&events), "failure:"), 1);
}

#[test]
fn emits_hook_replacement() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (generator, hooks) =
        recording_generator(Arc::clone(&events), Arc::new(Mutex::new(Vec::new())), None);
    hooks.set_per_chunk(true);
    hooks.rewrite.store(true, Ordering::Relaxed);
    let out = frames(body_of(
        &generator,
        source(
            vec![Ok(chat_chunk("one")), Ok(chat_chunk("two"))],
            JsonObject::new(),
        ),
        request(&[]),
        false,
    ));
    assert_eq!(out[0], chunk_frame(&expected_chunk("hooked", None)));
    assert_eq!(out[1], chunk_frame(&expected_chunk("hooked", None)));
    assert_eq!(count(&recorded(&events), "chunk:hooked"), 1);
}

#[test]
fn no_event_without_errors() {
    let (generator, _hooks) = fallback_generator();
    let hidden = fallback_hidden(None, &[]);
    let out = frames(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], hidden),
        include_request(),
        false,
    ));
    assert_eq!(
        out,
        [
            chunk_frame(&expected_chunk("x", None)),
            DONE_FRAME.to_owned()
        ]
    );
}

#[test]
fn uses_supplied_router_check() {
    let mut generator = plain_generator();
    generator.is_model_router = |model| model == "router";
    let out = frames(body_of(
        &generator,
        source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[("model", JsonValue::from("router"))]),
        false,
    ));
    assert_eq!(out[0], chunk_frame(&expected_chunk("x", None)));
}
