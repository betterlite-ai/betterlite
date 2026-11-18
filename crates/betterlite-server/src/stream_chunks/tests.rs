#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::set_default;

use super::fallback::{
    FallbackMeta, client_requested_model, fallback_event_frame, fallback_metadata,
    include_fallback_errors, is_positive_int_like,
};
use super::frames::{ends_with_frame_delimiter, pop_sse_frame, sse_data_frame};
use super::restamp::{
    RestampInput, RouterCheck, restamp_chunk_model, should_return_raw_model_name,
};
use super::serialize::{chunk_response_text, is_usage_artifact, serialize_chunk};
use super::{
    ChunkKind, DONE_FRAME, MAX_RAW_SSE_BUFFER_CHARS, SSE_FRAME_DELIMITERS, StreamChunk, StreamItem,
};

static ROUTER_CALLS: AtomicUsize = AtomicUsize::new(0);

fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON value")
}

fn object(text: &str) -> JsonObject {
    match value(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is not a JSON object")
        }
    }
}

fn typed_chunk(kind: ChunkKind, body: &str) -> StreamChunk {
    StreamChunk {
        body: object(body),
        kind,
        hidden: JsonObject::new(),
    }
}

fn chat_item(model: Option<&str>) -> StreamItem {
    let mut body = JsonObject::new();
    body.insert("id".to_owned(), Json::Text("chatcmpl-1".to_owned()));
    if let Some(text) = model {
        body.insert("model".to_owned(), Json::Text(text.to_owned()));
    }
    body.insert("choices".to_owned(), Json::List(Vec::new()));
    StreamItem::Chunk(StreamChunk {
        body,
        kind: ChunkKind::ModelStream,
        hidden: JsonObject::new(),
    })
}

fn item_model(item: &StreamItem) -> Option<&Json> {
    match item {
        StreamItem::Chunk(chunk) => chunk.body.get("model"),
        StreamItem::Bytes(_) | StreamItem::Text(_) => None,
    }
}

fn restamp_input<'request>(
    requested: &'request str,
    request: &'request JsonObject,
) -> RestampInput<'request> {
    RestampInput {
        requested_model: requested,
        request_data: request,
        fallback_attempted: false,
        fallback_model: None,
        is_model_router: never_router,
    }
}

fn never_router(_model: &str) -> bool {
    false
}

fn name_router(model: &str) -> bool {
    model == "requested-only"
}

fn counting_router(model: &str) -> bool {
    ROUTER_CALLS.fetch_add(1, Ordering::SeqCst);
    model == "requested-only"
}

fn hidden_headers(headers: &str) -> JsonObject {
    object(&format!(r#"{{"additional_headers": {headers}}}"#))
}

fn model_text(text: &str) -> Option<Json> {
    Some(Json::Text(text.to_owned()))
}

#[test]
fn client_model_prefers_requested_key() {
    let request = object(r#"{"_litellm_client_requested_model": "client", "model": "routed"}"#);
    assert_eq!(client_requested_model(&request), "client");
}

#[test]
fn client_model_falls_back_to_model() {
    let request = object(r#"{"model": "gpt-4o"}"#);
    assert_eq!(client_requested_model(&request), "gpt-4o");
    let null_request = object(r#"{"_litellm_client_requested_model": null, "model": "gpt-4o"}"#);
    assert_eq!(client_requested_model(&null_request), "gpt-4o");
    let number_request = object(r#"{"_litellm_client_requested_model": 7, "model": "gpt-4o"}"#);
    assert_eq!(client_requested_model(&number_request), "gpt-4o");
}

#[test]
fn client_model_empty_without_text() {
    assert_eq!(client_requested_model(&object("{}")), "");
    assert_eq!(client_requested_model(&object(r#"{"model": 5}"#)), "");
    assert_eq!(client_requested_model(&object(r#"{"model": null}"#)), "");
    assert_eq!(
        client_requested_model(&object(r#"{"_litellm_client_requested_model": []}"#)),
        ""
    );
}

#[rstest]
#[case::two("2")]
#[case::one("1")]
#[case::u64_max("18446744073709551615")]
fn int_like_accepts_json_integers(#[case] raw: &str) {
    assert!(is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::i64_min("-9223372036854775808")]
fn int_like_rejects_nonpositive_integers(#[case] raw: &str) {
    assert!(!is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[rstest]
#[case::truncated("2.7", true)]
#[case::whole("1.0", true)]
#[case::below_one("0.9", false)]
#[case::negative_fraction("-0.5", false)]
#[case::zero("0.0", false)]
fn int_like_truncates_json_floats(#[case] raw: &str, #[case] expected: bool) {
    assert_eq!(is_positive_int_like(Some(&value(raw))), expected, "{raw}");
}

#[rstest]
#[case::plain(r#""3""#)]
#[case::signed(r#"" +4 ""#)]
#[case::underscore(r#""1_0""#)]
#[case::padded(r#""007""#)]
#[case::signed_underscore(r#""+0_1""#)]
fn int_like_reads_python_int_text(#[case] raw: &str) {
    assert!(is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[rstest]
#[case::empty(r#""""#)]
#[case::space(r#"" ""#)]
#[case::decimal(r#""2.5""#)]
#[case::letters(r#""abc""#)]
#[case::leading_underscore(r#""_0""#)]
#[case::trailing_underscore(r#""0_""#)]
#[case::double_underscore(r#""0__0""#)]
#[case::sign_only(r#""+""#)]
#[case::minus_only(r#""-""#)]
#[case::hex(r#""0x10""#)]
#[case::inner_space(r#""1 0""#)]
#[case::unicode_digit(r#""١""#)]
#[case::zero(r#""0""#)]
#[case::negative_zero(r#""-0""#)]
fn int_like_rejects_bad_text(#[case] raw: &str) {
    assert!(!is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[rstest]
#[case::negative(r#""-3""#)]
#[case::padded_negative(r#"" -3 ""#)]
#[case::negative_underscore(r#""-1_0""#)]
fn int_like_rejects_negative_text(#[case] raw: &str) {
    assert!(!is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[rstest]
#[case::null("null")]
#[case::true_value("true")]
#[case::false_value("false")]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
#[case::filled_list("[1]")]
#[case::filled_object(r#"{"a": 1}"#)]
fn int_like_rejects_other_values(#[case] raw: &str) {
    assert!(!is_positive_int_like(Some(&value(raw))), "{raw}");
}

#[test]
fn int_like_rejects_missing_value() {
    assert!(!is_positive_int_like(None));
}

#[test]
fn include_errors_needs_both_switches() {
    let flagged = object(r#"{"include_fallback_errors": true}"#);
    let plain = object("{}");
    assert!(include_fallback_errors(true, &flagged));
    assert!(!include_fallback_errors(false, &flagged));
    assert!(!include_fallback_errors(true, &plain));
    assert!(!include_fallback_errors(false, &plain));
}

#[rstest]
#[case::text(r#"{"include_fallback_errors": "true"}"#)]
#[case::number(r#"{"include_fallback_errors": 1}"#)]
#[case::null(r#"{"include_fallback_errors": null}"#)]
#[case::list(r#"{"include_fallback_errors": [true]}"#)]
fn include_errors_requires_json_true(#[case] raw: &str) {
    assert!(!include_fallback_errors(true, &object(raw)), "{raw}");
}

#[test]
fn fallback_meta_needs_headers() {
    assert_eq!(fallback_metadata(&object("{}")), FallbackMeta::default());
    assert_eq!(
        fallback_metadata(&object(r#"{"additional_headers": null}"#)),
        FallbackMeta::default()
    );
    assert_eq!(
        fallback_metadata(&object(r#"{"additional_headers": "x"}"#)),
        FallbackMeta::default()
    );
}

#[rstest]
#[case::missing("{}")]
#[case::zero_text(r#"{"x-litellm-attempted-fallbacks": "0"}"#)]
#[case::float_text(r#"{"x-litellm-attempted-fallbacks": "2.5"}"#)]
#[case::zero_number(r#"{"x-litellm-attempted-fallbacks": 0}"#)]
#[case::small_number(r#"{"x-litellm-attempted-fallbacks": 0.5}"#)]
#[case::null(r#"{"x-litellm-attempted-fallbacks": null}"#)]
#[case::flag(r#"{"x-litellm-attempted-fallbacks": true}"#)]
fn fallback_meta_needs_positive_attempts(#[case] headers: &str) {
    assert_eq!(
        fallback_metadata(&hidden_headers(headers)),
        FallbackMeta::default(),
        "{headers}"
    );
}

#[test]
fn fallback_meta_counts_truncated_attempts() {
    let number = hidden_headers(r#"{"x-litellm-attempted-fallbacks": 2.5}"#);
    assert_eq!(
        fallback_metadata(&number),
        FallbackMeta {
            attempted: true,
            model: None,
            errors: Vec::new(),
        }
    );
    let text = hidden_headers(r#"{"x-litellm-attempted-fallbacks": "3"}"#);
    assert!(fallback_metadata(&text).attempted);
}

#[test]
fn fallback_meta_reads_model_group() {
    let named = hidden_headers(
        r#"{"x-litellm-attempted-fallbacks": 1, "x-litellm-model-group": "gpt-4o"}"#,
    );
    let meta = fallback_metadata(&named);
    assert!(meta.attempted);
    assert_eq!(meta.model.as_deref(), Some("gpt-4o"));
}

#[rstest]
#[case::empty(r#""""#)]
#[case::number("5")]
#[case::null("null")]
#[case::flag("true")]
fn fallback_meta_ignores_unusable_model_group(#[case] raw: &str) {
    let headers =
        format!(r#"{{"x-litellm-attempted-fallbacks": 1, "x-litellm-model-group": {raw}}}"#);
    let meta = fallback_metadata(&hidden_headers(&headers));
    assert!(meta.attempted, "{raw}");
    assert_eq!(meta.model, None, "{raw}");
}

#[test]
fn fallback_meta_reads_error_lists() {
    let listed = hidden_headers(
        r#"{"x-litellm-attempted-fallbacks": 1, "x-litellm-fallback-errors": [{"message": "boom"}, 5, null, {"message": "again"}]}"#,
    );
    assert_eq!(
        fallback_metadata(&listed).errors,
        vec![
            object(r#"{"message": "boom"}"#),
            object(r#"{"message": "again"}"#),
        ]
    );
    let encoded = hidden_headers(
        r#"{"x-litellm-attempted-fallbacks": 1, "x-litellm-fallback-errors": "[{\"message\": \"boom\"}]"}"#,
    );
    assert_eq!(
        fallback_metadata(&encoded).errors,
        vec![object(r#"{"message": "boom"}"#)]
    );
}

#[rstest]
#[case::broken_text(r#""not json""#)]
#[case::object_text(r#""{\"a\": 1}""#)]
#[case::number_text(r#""[5]""#)]
#[case::number("7")]
#[case::null("null")]
fn fallback_meta_ignores_bad_errors(#[case] raw: &str) {
    let headers =
        format!(r#"{{"x-litellm-attempted-fallbacks": 1, "x-litellm-fallback-errors": {raw}}}"#);
    assert!(
        fallback_metadata(&hidden_headers(&headers))
            .errors
            .is_empty()
    );
}

#[test]
fn fallback_meta_attempted_without_model() {
    let meta = fallback_metadata(&hidden_headers(
        r#"{"x-litellm-attempted-fallbacks": " 2 "}"#,
    ));
    assert!(meta.attempted);
    assert_eq!(meta.model, None);
    assert!(meta.errors.is_empty());
}

#[test]
fn fallback_event_without_model() {
    let frame = fallback_event_frame(None, &[], 1_700_000_000);
    let expected = concat!(
        r#"data: {"id":"litellm-fallback-metadata","object":"chat.completion.chunk","#,
        r#""created":1700000000,"model":"","choices":[],"#,
        r#""litellm_fallback":{"fallback_model":null,"errors":[]}}"#,
        "\n\n"
    );
    assert_eq!(frame, expected);
}

#[test]
fn fallback_event_carries_model_and_errors() {
    let errors = vec![object(r#"{"message": "boom", "type": "RateLimitError"}"#)];
    let frame = fallback_event_frame(Some("gpt-4o"), &errors, 7);
    let expected = concat!(
        r#"data: {"id":"litellm-fallback-metadata","object":"chat.completion.chunk","#,
        r#""created":7,"model":"gpt-4o","choices":[],"#,
        r#""litellm_fallback":{"fallback_model":"gpt-4o","#,
        r#""errors":[{"message":"boom","type":"RateLimitError"}]}}"#,
        "\n\n"
    );
    assert_eq!(frame, expected);
}

#[test]
fn raw_name_reads_both_metadata_maps() {
    assert!(should_return_raw_model_name(&object(
        r#"{"metadata": {"_complexity_router_return_raw_model_name": true}}"#
    )));
    assert!(should_return_raw_model_name(&object(
        r#"{"litellm_metadata": {"_complexity_router_return_raw_model_name": true}}"#
    )));
    assert!(!should_return_raw_model_name(&object(
        r#"{"metadata": {"_complexity_router_return_raw_model_name": false}, "litellm_metadata": {}}"#
    )));
}

#[rstest]
#[case::empty("{}")]
#[case::null_metadata(r#"{"metadata": null}"#)]
#[case::text_metadata(r#"{"metadata": "true"}"#)]
#[case::text_flag(r#"{"metadata": {"_complexity_router_return_raw_model_name": "true"}}"#)]
#[case::number_flag(r#"{"metadata": {"_complexity_router_return_raw_model_name": 1}}"#)]
#[case::other_key(r#"{"litellm_metadata": {"other": true}}"#)]
fn raw_name_needs_json_true(#[case] raw: &str) {
    assert!(!should_return_raw_model_name(&object(raw)), "{raw}");
}

#[test]
fn restamp_sets_client_model() {
    let request = object("{}");
    let mut item = chat_item(Some("provider/x"));
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), false);
    assert!(logged);
    assert_eq!(item_model(&item), model_text("gpt-4o").as_ref());
}

#[test]
fn restamp_adds_missing_model() {
    let request = object("{}");
    let mut item = chat_item(None);
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), false);
    assert!(logged);
    assert_eq!(item_model(&item), model_text("gpt-4o").as_ref());
}

#[test]
fn restamp_leaves_matching_model() {
    let request = object("{}");
    let mut item = chat_item(Some("gpt-4o"));
    let before = item.clone();
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), false);
    assert!(!logged);
    assert_eq!(item, before);
}

#[test]
fn restamp_keeps_logged_flag() {
    let request = object("{}");
    let mut item = chat_item(Some("provider/x"));
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), true);
    assert!(logged);
    assert_eq!(item_model(&item), model_text("gpt-4o").as_ref());
}

#[test]
fn restamp_keeps_flag_on_matching_model() {
    let request = object("{}");
    let mut item = chat_item(Some("gpt-4o"));
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), true);
    assert!(logged);
    assert_eq!(item_model(&item), model_text("gpt-4o").as_ref());
}

#[test]
fn restamp_skips_raw_name_requests() {
    let request = object(r#"{"metadata": {"_complexity_router_return_raw_model_name": true}}"#);
    let mut item = chat_item(Some("provider/x"));
    let before = item.clone();
    assert!(!restamp_chunk_model(
        &mut item,
        &restamp_input("gpt-4o", &request),
        false
    ));
    assert_eq!(item, before);
}

#[test]
fn restamp_skips_empty_target() {
    let request = object("{}");
    let mut item = chat_item(Some("provider/x"));
    let before = item.clone();
    assert!(!restamp_chunk_model(
        &mut item,
        &restamp_input("", &request),
        false
    ));
    let mut fallback = restamp_input("gpt-4o", &request);
    fallback.fallback_attempted = true;
    fallback.fallback_model = Some("");
    let mut empty_fallback = chat_item(Some("provider/x"));
    assert!(!restamp_chunk_model(&mut empty_fallback, &fallback, false));
    fallback.fallback_model = None;
    let mut missing_fallback = chat_item(Some("provider/x"));
    assert!(!restamp_chunk_model(
        &mut missing_fallback,
        &fallback,
        false
    ));
    assert_eq!(empty_fallback, before);
    assert_eq!(missing_fallback, before);
    assert_eq!(item, before);
}

#[test]
fn restamp_skips_bytes_and_text() {
    let request = object("{}");
    let mut bytes = StreamItem::Bytes(Bytes::from_static(b"data: chunk"));
    let mut text = StreamItem::Text("data: chunk".to_owned());
    assert!(!restamp_chunk_model(
        &mut bytes,
        &restamp_input("gpt-4o", &request),
        false
    ));
    assert!(!restamp_chunk_model(
        &mut text,
        &restamp_input("gpt-4o", &request),
        false
    ));
    assert_eq!(bytes, StreamItem::Bytes(Bytes::from_static(b"data: chunk")));
    assert_eq!(text, StreamItem::Text("data: chunk".to_owned()));
}

#[test]
fn restamp_skips_model_router_before_fallback() {
    ROUTER_CALLS.store(0, Ordering::SeqCst);
    let request = object("{}");
    let mut input = restamp_input("requested-only", &request);
    let counting: RouterCheck = counting_router;
    input.is_model_router = counting;
    let mut item = chat_item(Some("provider/x"));
    assert!(!restamp_chunk_model(&mut item, &input, false));
    assert_eq!(ROUTER_CALLS.load(Ordering::SeqCst), 1);
    input.fallback_attempted = true;
    input.fallback_model = Some("fallback-x");
    let logged = restamp_chunk_model(&mut item, &input, false);
    assert!(logged);
    assert_eq!(item_model(&item), model_text("fallback-x").as_ref());
    assert_eq!(ROUTER_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn restamp_router_check_gets_requested_model() {
    let request = object("{}");
    let mut input = restamp_input("requested-only", &request);
    let named: RouterCheck = name_router;
    input.is_model_router = named;
    let mut item = chat_item(Some("provider/x"));
    assert!(!restamp_chunk_model(&mut item, &input, false));
    let plain = object("{}");
    let mut other = restamp_input("gpt-4o", &plain);
    other.is_model_router = named;
    let mut second = chat_item(Some("provider/x"));
    assert!(restamp_chunk_model(&mut second, &other, false));
    assert_eq!(item_model(&second), model_text("gpt-4o").as_ref());
}

#[test]
fn restamp_skips_fastest_response_before_fallback() {
    let request = object(r#"{"fastest_response": true}"#);
    let mut item = chat_item(Some("provider/x"));
    let before = item.clone();
    assert!(!restamp_chunk_model(
        &mut item,
        &restamp_input("gpt-4o", &request),
        false
    ));
    assert_eq!(item, before);
    let mut fallback = restamp_input("gpt-4o", &request);
    fallback.fallback_attempted = true;
    fallback.fallback_model = Some("fallback-x");
    assert!(restamp_chunk_model(&mut item, &fallback, false));
    assert_eq!(item_model(&item), model_text("fallback-x").as_ref());
}

#[test]
fn restamp_stamps_when_not_fastest_response() {
    let request = object(r#"{"fastest_response": false}"#);
    let mut item = chat_item(Some("provider/x"));
    assert!(restamp_chunk_model(
        &mut item,
        &restamp_input("gpt-4o", &request),
        false
    ));
}

#[rstest]
#[case::stream(ChunkKind::ModelStream)]
#[case::full(ChunkKind::ModelFull)]
#[case::event(ChunkKind::Event)]
#[case::plain(ChunkKind::Plain)]
fn restamp_stamps_every_chunk_kind(#[case] kind: ChunkKind) {
    let request = object("{}");
    let mut item = StreamItem::Chunk(typed_chunk(kind, r#"{"type": "response.created"}"#));
    let logged = restamp_chunk_model(&mut item, &restamp_input("gpt-4o", &request), false);
    assert!(logged, "{kind:?}");
    assert_eq!(item_model(&item), model_text("gpt-4o").as_ref(), "{kind:?}");
}

#[test]
fn restamp_uses_fallback_model_after_fallback() {
    let request = object("{}");
    let mut item = chat_item(Some("provider/x"));
    let mut input = restamp_input("gpt-4o", &request);
    input.fallback_attempted = true;
    input.fallback_model = Some("fallback-x");
    assert!(restamp_chunk_model(&mut item, &input, false));
    assert_eq!(item_model(&item), model_text("fallback-x").as_ref());
}

#[test]
fn restamp_logs_mismatch_once() {
    let log = NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("log handle")))
        .finish();
    let request = object(r#"{"litellm_call_id": "call-1"}"#);
    let input = restamp_input("gpt-4o", &request);
    let (logged, second_logged) = {
        let _guard = set_default(subscriber);
        let mut item = chat_item(Some("provider/x"));
        let logged = restamp_chunk_model(&mut item, &input, false);
        let mut second = chat_item(Some("provider/y"));
        let second_logged = restamp_chunk_model(&mut second, &input, logged);
        assert_eq!(item_model(&second), model_text("gpt-4o").as_ref());
        (logged, second_logged)
    };
    assert!(logged && second_logged);
    let output = fs::read_to_string(log.path()).expect("log output");
    assert_eq!(
        output.matches("streaming chunk model mismatch").count(),
        1,
        "{output}"
    );
    assert!(output.contains("litellm_call_id=call-1"), "{output}");
    assert!(output.contains("target=gpt-4o"), "{output}");
    assert!(output.contains("downstream=provider/x"), "{output}");
    assert!(output.contains("fallback_was_attempted=false"), "{output}");
    assert!(
        output.contains("Overriding chunk model to target."),
        "{output}"
    );
}

#[test]
fn serialize_drops_null_fields() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chunk", "object": "chat.completion.chunk", "created": 1, "model": null,
            "choices": [{"index": 0, "delta": {"content": null, "role": "assistant"}, "finish_reason": null}],
            "system_fingerprint": null}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"id":"chunk","object":"chat.completion.chunk","created":1,"#,
            r#""choices":[{"index":0,"delta":{"role":"assistant"}}]}"#
        )
    );
}

#[test]
fn serialize_keeps_null_array_items() {
    let chunk = typed_chunk(
        ChunkKind::ModelFull,
        r#"{"choices": [null, {"message": {"content": null, "role": "assistant"}, "index": 0}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        r#"{"choices":[null,{"message":{"role":"assistant"},"index":0}]}"#
    );
}

#[test]
fn serialize_plain_keeps_nulls() {
    let chunk = typed_chunk(
        ChunkKind::Plain,
        r#"{"a": null, "b": [null, {"c": null}], "d": {"e": null, "f": 1}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        r#"{"a":null,"b":[null,{"c":null}],"d":{"e":null,"f":1}}"#
    );
}

#[test]
fn serialize_event_keeps_dictionary_nulls() {
    // the carrier model drops its own null field and keeps the extra's dictionary as it came
    let chunk = typed_chunk(
        ChunkKind::Event,
        r#"{"type": "response.created", "nullable": null, "response": {"id": null, "status": "in_progress"}}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        r#"{"type":"response.created","response":{"id":null,"status":"in_progress"}}"#
    );
}

#[test]
fn serialize_keeps_provider_field_nulls() {
    let stream = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chunk", "model": "gpt-4o", "choices": [{"index": 0, "delta": {"content": "hi", "provider_specific_fields": {"nullable": null, "nested": {"a": null, "b": 1}}}}], "provider_specific_fields": {"nullable": null, "nested": [null, {"c": null}]}}"#,
    );
    assert_eq!(
        serialize_chunk(&stream),
        r#"{"id":"chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hi","provider_specific_fields":{"nullable":null,"nested":{"a":null,"b":1}}}}],"provider_specific_fields":{"nullable":null,"nested":[null,{"c":null}]}}"#
    );
    let full = typed_chunk(
        ChunkKind::ModelFull,
        r#"{"choices": [{"index": 0, "provider_specific_fields": {"nullable": null}, "message": {"content": "hi", "provider_specific_fields": {"nested": {"a": null}}}}]}"#,
    );
    assert_eq!(
        serialize_chunk(&full),
        r#"{"choices":[{"index":0,"provider_specific_fields":{"nullable":null},"message":{"content":"hi","provider_specific_fields":{"nested":{"a":null}}}}]}"#
    );
}

#[test]
fn serialize_drops_nested_model_nulls() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chunk", "choices": [{"index": 0, "delta": {"content": "hi", "tool_calls": [{"index": 0, "type": "function", "function": {"name": null, "arguments": "{}"}}], "thinking_blocks": [{"type": "thinking", "thinking": "t", "signature": null}]}, "logprobs": {"content": [{"token": "hi", "bytes": null, "logprob": -0.1, "top_logprobs": [{"token": "hi", "bytes": null, "logprob": -0.1}]}]}}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        r#"{"id":"chunk","choices":[{"index":0,"delta":{"content":"hi","tool_calls":[{"index":0,"type":"function","function":{"arguments":"{}"}}],"thinking_blocks":[{"type":"thinking","thinking":"t"}]},"logprobs":{"content":[{"token":"hi","logprob":-0.1,"top_logprobs":[{"token":"hi","logprob":-0.1}]}]}}]}"#
    );
}

#[test]
fn serialize_keeps_stream_reasoning_items() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"id": "chunk", "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": "",
                "reasoning_items": [{"type": "reasoning", "id": "rs-1", "encrypted_content": "cipher",
                    "summary": [{"type": "summary_text", "text": "thought"}]}]}}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"id":"chunk","model":"gpt-4o","choices":[{"index":0,"#,
            r#""delta":{"role":"assistant","content":"","reasoning_items":[{"type":"reasoning","#,
            r#""id":"rs-1","encrypted_content":"cipher","summary":[{"type":"summary_text","text":"thought"}]}]}}]}"#
        )
    );
}

#[test]
fn serialize_full_reasoning_items_drop_nulls() {
    let chunk = typed_chunk(
        ChunkKind::ModelFull,
        r#"{"choices": [{"index": 0, "message": {"role": "assistant", "content": "hi",
            "reasoning_items": [{"type": "reasoning", "id": "rs-1", "encrypted_content": null, "summary": []}]}}]}"#,
    );
    assert_eq!(
        serialize_chunk(&chunk),
        concat!(
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi","#,
            r#""reasoning_items":[{"type":"reasoning","id":"rs-1","summary":[]}]}}]}"#
        )
    );
}

#[rstest]
#[case::no_key(r#"{"id": "chunk"}"#)]
#[case::empty_list(r#"{"id": "chunk", "choices": []}"#)]
#[case::null_list(r#"{"id": "chunk", "choices": null}"#)]
#[case::null_fields(r#"{"id": "chunk", "choices": [], "provider_specific_fields": null}"#)]
fn usage_artifact_true_without_choices(#[case] body: &str) {
    assert!(
        is_usage_artifact(&typed_chunk(ChunkKind::ModelStream, body)),
        "{body}"
    );
}

#[rstest]
#[case::bare_choice(r#"{"choices": [{}]}"#)]
#[case::indexed(r#"{"choices": [{"index": 0}]}"#)]
#[case::empty_delta(r#"{"choices": [{"delta": {}}]}"#)]
#[case::null_fields(
    r#"{"choices": [{"delta": {"content": null}, "finish_reason": null, "logprobs": null}]}"#
)]
#[case::null_delta(r#"{"choices": [{"delta": null}]}"#)]
fn usage_artifact_true_for_empty_choices(#[case] body: &str) {
    assert!(
        is_usage_artifact(&typed_chunk(ChunkKind::ModelStream, body)),
        "{body}"
    );
}

#[rstest]
#[case::content(r#"{"choices": [{"delta": {"content": "hi"}}]}"#)]
#[case::empty_content(r#"{"choices": [{"delta": {"content": ""}}]}"#)]
#[case::role(r#"{"choices": [{"delta": {"role": "assistant"}}]}"#)]
#[case::finish_reason(r#"{"choices": [{"finish_reason": "stop"}]}"#)]
#[case::logprobs(r#"{"choices": [{"logprobs": {}}]}"#)]
#[case::delta_text(r#"{"choices": [{"delta": "text"}]}"#)]
#[case::choice_text(r#"{"choices": ["choice"]}"#)]
#[case::mixed_choices(r#"{"choices": [{"index": 0}, {"delta": {"content": "hi"}}]}"#)]
#[case::text_choices(r#"{"choices": "x"}"#)]
#[case::object_choices(r#"{"choices": {"a": 1}}"#)]
fn usage_artifact_false_with_payload(#[case] body: &str) {
    assert!(
        !is_usage_artifact(&typed_chunk(ChunkKind::ModelStream, body)),
        "{body}"
    );
}

#[rstest]
#[case::full(ChunkKind::ModelFull)]
#[case::event(ChunkKind::Event)]
#[case::plain(ChunkKind::Plain)]
fn usage_artifact_false_for_other_kinds(#[case] kind: ChunkKind) {
    assert!(
        !is_usage_artifact(&typed_chunk(kind, r#"{"choices": []}"#)),
        "{kind:?}"
    );
}

#[rstest]
#[case::object(r#"{"a": 1}"#)]
#[case::empty_object("{}")]
#[case::flag("false")]
#[case::number("0")]
#[case::text(r#""x""#)]
fn usage_artifact_false_with_provider_fields(#[case] raw: &str) {
    let body = format!(r#"{{"choices": [], "provider_specific_fields": {raw}}}"#);
    assert!(
        !is_usage_artifact(&typed_chunk(ChunkKind::ModelStream, &body)),
        "{raw}"
    );
}

#[test]
fn response_text_joins_stream_content() {
    let chunk = typed_chunk(
        ChunkKind::ModelStream,
        r#"{"choices": [{"delta": {"content": "He"}}, {"delta": {"content": "llo"}},
            {"delta": {"content": null}}, {"delta": {"role": "assistant"}}]}"#,
    );
    assert_eq!(chunk_response_text(&chunk), "Hello");
}

#[test]
fn response_text_reads_message_content() {
    let chunk = typed_chunk(
        ChunkKind::ModelFull,
        r#"{"choices": [{"message": {"content": "hi", "role": "assistant"}},
            {"message": {"role": "assistant"}}, {"index": 0}]}"#,
    );
    assert_eq!(chunk_response_text(&chunk), "hi");
}

#[rstest]
#[case::number(r#"{"content": 5}"#, "5")]
#[case::list(r#"{"content": [1, 2]}"#, "[1,2]")]
#[case::object(r#"{"content": {"a": 1}}"#, r#"{"a":1}"#)]
#[case::flag(r#"{"content": false}"#, "false")]
fn response_text_writes_other_content_as_json(#[case] raw: &str, #[case] expected: &str) {
    let body = format!(r#"{{"choices": [{{"delta": {raw}}}]}}"#);
    assert_eq!(
        chunk_response_text(&typed_chunk(ChunkKind::ModelStream, &body)),
        expected
    );
}

#[rstest]
#[case::missing_choices("{}")]
#[case::null_choices(r#"{"choices": null}"#)]
#[case::text_choices(r#"{"choices": "x"}"#)]
#[case::bare_choice(r#"{"choices": [{"index": 0}]}"#)]
#[case::text_choice(r#"{"choices": ["choice"]}"#)]
#[case::null_content(r#"{"choices": [{"delta": {"content": null}}]}"#)]
fn response_text_skips_empty_shapes(#[case] body: &str) {
    assert_eq!(
        chunk_response_text(&typed_chunk(ChunkKind::ModelStream, body)),
        "",
        "{body}"
    );
}

#[rstest]
#[case::event(ChunkKind::Event)]
#[case::plain(ChunkKind::Plain)]
fn response_text_empty_for_other_kinds(#[case] kind: ChunkKind) {
    let body = r#"{"choices": [{"delta": {"content": "hi"}, "message": {"content": "hi"}}]}"#;
    assert_eq!(
        chunk_response_text(&typed_chunk(kind, body)),
        "",
        "{kind:?}"
    );
}

#[test]
fn sse_data_frame_wraps_payload() {
    assert_eq!(sse_data_frame(r#"{"a":1}"#), "data: {\"a\":1}\n\n");
    assert_eq!(sse_data_frame(""), "data: \n\n");
    assert_eq!(sse_data_frame("[DONE]"), DONE_FRAME);
}

#[test]
fn sse_constants_match_litellm() {
    assert_eq!(DONE_FRAME, "data: [DONE]\n\n");
    assert_eq!(SSE_FRAME_DELIMITERS, ["\r\n\r\n", "\n\n", "\r\r"]);
    assert_eq!(MAX_RAW_SSE_BUFFER_CHARS, 8 * 1024 * 1024);
}

#[rstest]
#[case::crlf("x\r\n\r\n")]
#[case::lf("x\n\n")]
#[case::cr("x\r\r")]
#[case::only_delimiter("\n\n")]
fn ends_with_frame_delimiter_accepts(#[case] text: &str) {
    assert!(ends_with_frame_delimiter(text), "{text:?}");
}

#[rstest]
#[case::empty("")]
#[case::plain("x")]
#[case::half_lf("x\n")]
#[case::half_cr("x\r")]
#[case::prefix("\n\nx")]
#[case::trailing("x\n\n\r")]
fn ends_with_frame_delimiter_rejects(#[case] text: &str) {
    assert!(!ends_with_frame_delimiter(text), "{text:?}");
}

#[test]
fn pop_frame_without_delimiter() {
    assert_eq!(pop_sse_frame("data: x"), (None, "data: x".to_owned()));
    assert_eq!(pop_sse_frame(""), (None, String::new()));
}

#[test]
fn pop_frame_keeps_delimiter_and_rest() {
    assert_eq!(
        pop_sse_frame("data: a\n\nrest"),
        (Some("data: a\n\n".to_owned()), "rest".to_owned())
    );
    assert_eq!(
        pop_sse_frame("\r\n\r\ntail"),
        (Some("\r\n\r\n".to_owned()), "tail".to_owned())
    );
    assert_eq!(
        pop_sse_frame("a\r\rb"),
        (Some("a\r\r".to_owned()), "b".to_owned())
    );
}

#[test]
fn pop_frame_picks_earliest_delimiter() {
    assert_eq!(
        pop_sse_frame("a\r\rb\n\nc"),
        (Some("a\r\r".to_owned()), "b\n\nc".to_owned())
    );
    assert_eq!(
        pop_sse_frame("ab\n\ncd\r\n\r\nef"),
        (Some("ab\n\n".to_owned()), "cd\r\n\r\nef".to_owned())
    );
    assert_eq!(
        pop_sse_frame("ab\r\n\r\ncd\n\nef"),
        (Some("ab\r\n\r\n".to_owned()), "cd\n\nef".to_owned())
    );
}

#[rstest]
fn replays_the_sse_frame_corpus(#[files("fuzz/corpus/sse_frame/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("corpus seed is readable");
    let buffer = String::from_utf8_lossy(&bytes);
    let (frame, rest) = pop_sse_frame(&buffer);
    if let Some(frame) = frame {
        assert!(
            SSE_FRAME_DELIMITERS
                .iter()
                .any(|delimiter| frame.ends_with(*delimiter)),
            "{}",
            path.display()
        );
        assert_eq!(
            format!("{frame}{rest}"),
            buffer.to_string(),
            "{}",
            path.display()
        );
    } else {
        assert_eq!(rest, buffer.to_string(), "{}", path.display());
    }
}

#[test]
fn hidden_params_returns_chunk_map() {
    let mut chunk = typed_chunk(ChunkKind::ModelStream, "{}");
    chunk
        .hidden
        .insert("additional_headers".to_owned(), value("{}"));
    let item = StreamItem::Chunk(chunk);
    let hidden = item.hidden_params();
    assert_eq!(
        hidden.and_then(|fields| fields.get("additional_headers")),
        Some(&value("{}"))
    );
    assert_eq!(StreamItem::Bytes(Bytes::new()).hidden_params(), None);
    assert_eq!(StreamItem::Text(String::new()).hidden_params(), None);
}
