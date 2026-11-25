#![cfg(test)]

mod fixtures;
mod replay;

use std::iter;
use std::sync::{Arc, Mutex};

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::{Map, Value};

use self::fixtures::support::{
    Entry, FIXED_CLOCK, FIXED_TIMESTAMP, FIXED_UUID, FakeChunks, FakeError, FakeEvents,
    FakeExecution, RecordingProcessing, StreamParam, call, caller_with_identity_extras,
    chat_response, finish_of, hidden, hidden_member_of, id_of, kwargs, post_stream,
    post_stream_missing_record, post_stream_named, responses_body, sse, stamps, terminal_event,
    text_of,
};
use super::error::{BridgeStreamError, DispatchError};
use super::handler::completion;
use super::input::{json_mode, resolve_stream_flag};
use super::port::{
    Answer, BridgeOutput, BridgeResponse, BridgeStream, ChatResponse, ResponsePayload,
};
use super::stream::{StreamConfig, TranslatedStream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_result::records::ModelResponse;

fn optional_params(mut fields: IndexMap<String, Json>, params: &str) -> IndexMap<String, Json> {
    let parsed = serde_json::from_str::<Json>(params).unwrap();
    fields.insert(String::from("optional_params"), parsed);
    fields
}

fn without(mut fields: IndexMap<String, Json>, name: &str) -> IndexMap<String, Json> {
    fields.shift_remove(name);
    fields
}

fn with(mut fields: IndexMap<String, Json>, name: &str, value: Json) -> IndexMap<String, Json> {
    fields.insert(String::from(name), value);
    fields
}

fn streaming() -> IndexMap<String, Json> {
    with(kwargs("gpt-4o-mini", "openai"), "stream", Json::Bool(true))
}

fn created_event() -> Vec<u8> {
    sse(r#"{"type":"response.created","response":{"id":"resp_a"}}"#)
}

fn delta_event(text: &str) -> Vec<u8> {
    sse(&format!(
        r#"{{"type":"response.output_text.delta","delta":"{text}"}}"#
    ))
}

fn padding_line() -> Vec<u8> {
    b"event: response.created".to_vec()
}

fn frames() -> Vec<Vec<u8>> {
    vec![
        created_event(),
        padding_line(),
        delta_event("he"),
        delta_event("llo"),
        sse(&terminal_event("hello")),
    ]
}

fn terminal_frame(kind: &str, text: &str) -> Vec<u8> {
    sse(&format!(
        r#"{{"type":"{kind}","response":{}}}"#,
        responses_body(text)
    ))
}

fn events(lines: Vec<Vec<u8>>, hidden_pairs: &[(&str, &str)]) -> Answer {
    Answer::Events(Box::new(FakeEvents::new(lines, hidden(hidden_pairs))))
}

fn typed_response(text: &str) -> BridgeResponse {
    let parsed = serde_json::from_str::<ResponsesApiResponse>(&responses_body(text)).unwrap();
    BridgeResponse {
        payload: ResponsePayload::Typed(Box::new(parsed)),
        existing_hidden: Map::new(),
        stream_hidden: Map::new(),
    }
}

fn expect_ok(result: Result<BridgeOutput, DispatchError<FakeError>>) -> BridgeOutput {
    match result {
        Ok(output) => output,
        Err(error) => panic!("the bridge failed: {error}"),
    }
}

fn expect_error<T>(result: Result<T, DispatchError<FakeError>>) -> String {
    match result {
        Ok(_) => panic!("the bridge answered instead of failing"),
        Err(error) => error.to_string(),
    }
}

fn expect_response(output: BridgeOutput) -> Box<ModelResponse> {
    match output {
        BridgeOutput::Response(record) => record,
        BridgeOutput::Chat(_) | BridgeOutput::Stream(_) => panic!("expected a chat response"),
    }
}

fn expect_chat(output: BridgeOutput) -> Box<ChatResponse> {
    match output {
        BridgeOutput::Chat(chat) => chat,
        BridgeOutput::Response(_) | BridgeOutput::Stream(_) => panic!("expected a chat response"),
    }
}

fn expect_stream(output: BridgeOutput) -> BridgeStream {
    match output {
        BridgeOutput::Stream(stream) => stream,
        BridgeOutput::Response(_) | BridgeOutput::Chat(_) => panic!("expected a stream"),
    }
}

async fn drain(mut stream: BridgeStream) -> Vec<Json> {
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        chunks.push(chunk);
    }
    chunks
}

fn joined(chunks: &[Json]) -> String {
    chunks.iter().filter_map(text_of).collect()
}

fn finishes(chunks: &[Json]) -> Vec<String> {
    chunks.iter().filter_map(finish_of).collect()
}

fn ids(chunks: &[Json]) -> Vec<String> {
    chunks.iter().filter_map(id_of).collect()
}

fn created_of(chunk: &Json) -> Option<Json> {
    match chunk {
        Json::Object(fields) => fields.get("created").cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn optional_stream(value: Option<Json>) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    if let Some(requested) = value {
        fields.insert(String::from("stream"), requested);
    }
    fields
}

fn sdk_stream(value: Value) -> IndexMap<String, Value> {
    let mut fields = IndexMap::new();
    fields.insert(String::from("stream"), value);
    fields
}

#[rstest]
#[case::model("model")]
#[case::custom_llm_provider("custom_llm_provider")]
#[case::messages("messages")]
#[case::optional_params("optional_params")]
#[case::litellm_params("litellm_params")]
#[case::headers("headers")]
#[case::model_response("model_response")]
#[case::logging_obj("logging_obj")]
#[tokio::test]
async fn validation_names_the_first_missing_field_and_reaches_no_provider(#[case] field: &str) {
    let fields = without(kwargs("gpt-4o-mini", "openai"), field);
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let text = expect_error(completion(&execution, call(&fields)).await);
    assert_eq!(text, format!("{field} is required"));
    assert!(execution.observations().is_empty());
}

#[tokio::test]
async fn the_first_missing_field_is_the_one_named() {
    let fields = without(without(kwargs("gpt-4o-mini", "openai"), "model"), "headers");
    let text = expect_error(completion(&FakeExecution::failing(), call(&fields)).await);
    assert_eq!(text, "model is required");
}

#[rstest]
#[case::model("model", Json::Number(serde_json::Number::from(5_i32)))]
#[case::custom_llm_provider("custom_llm_provider", Json::Bool(true))]
#[case::messages("messages", Json::Text(String::from("hi")))]
#[case::optional_params("optional_params", Json::List(Vec::new()))]
#[case::litellm_params("litellm_params", Json::Text(String::from("none")))]
#[case::headers("headers", Json::Null)]
#[case::model_response("model_response", Json::Text(String::from("chat")))]
#[case::logging_obj("logging_obj", Json::Bool(false))]
#[tokio::test]
async fn validation_rejects_a_required_field_of_the_wrong_type(
    #[case] field: &str,
    #[case] value: Json,
) {
    let fields = with(kwargs("gpt-4o-mini", "openai"), field, value);
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let text = expect_error(completion(&execution, call(&fields)).await);
    assert_eq!(text, format!("{field} is required"));
}

#[tokio::test]
async fn a_headers_value_that_is_not_a_string_is_rejected() {
    let headers = r#"{"x-test":5}"#;
    let fields = with(
        kwargs("gpt-4o-mini", "openai"),
        "headers",
        serde_json::from_str(headers).unwrap(),
    );
    let text = expect_error(completion(&FakeExecution::failing(), call(&fields)).await);
    assert_eq!(text, "headers must be a map of strings");
}

#[tokio::test]
async fn a_message_that_is_not_an_object_is_rejected_by_the_mapping() {
    let fields = with(
        kwargs("gpt-4o-mini", "openai"),
        "messages",
        Json::List(vec![Json::Number(serde_json::Number::from(5_i32))]),
    );
    let text = expect_error(completion(&FakeExecution::failing(), call(&fields)).await);
    assert!(text.contains("invalid type"), "{text}");
}

#[tokio::test]
async fn empty_required_containers_are_accepted() {
    let base = kwargs("", "");
    let with_messages = with(base, "messages", Json::List(Vec::new()));
    let with_optional = with(
        with_messages,
        "optional_params",
        Json::Object(IndexMap::new()),
    );
    let fields = with(with_optional, "headers", Json::Object(IndexMap::new()));
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let output = expect_ok(completion(&execution, call(&fields)).await);
    assert_eq!(expect_chat(output), chat_response());
    assert_eq!(execution.observations()[0].model, "/");
}

#[tokio::test]
async fn encoding_is_optional_and_defaults_to_null() {
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    expect_ok(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    let fields = with(kwargs("gpt-4o-mini", "openai"), "encoding", Json::Null);
    expect_ok(
        completion(
            &FakeExecution::new(Answer::Chat(chat_response())),
            call(&fields),
        )
        .await,
    );
}

#[tokio::test]
async fn sync_passes_the_client_while_async_omits_it() {
    let fields = kwargs("gpt-4o-mini", "openai");
    let synchronous = FakeExecution::new(Answer::Chat(chat_response()));
    expect_ok(completion(&synchronous, call(&fields)).await);
    let async_fields = with(fields, "acompletion", Json::Bool(true));
    let asynchronous = FakeExecution::new(Answer::Chat(chat_response()));
    expect_ok(completion(&asynchronous, call(&async_fields)).await);
    assert!(synchronous.observations()[0].client);
    assert!(!asynchronous.observations()[0].client);
    assert_eq!(synchronous.observations()[0].model, "openai/gpt-4o-mini");
    assert_eq!(
        synchronous.observations()[0].provider.as_deref(),
        Some("openai")
    );
}

#[rstest]
#[case::boolean_true(Json::Bool(true), Entry::Aresponses, false)]
#[case::boolean_false(Json::Bool(false), Entry::Responses, true)]
#[case::text_true(Json::Text(String::from("true")), Entry::Responses, true)]
#[case::number_one(Json::Number(serde_json::Number::from(1_i32)), Entry::Responses, true)]
#[case::null(Json::Null, Entry::Responses, true)]
#[tokio::test]
async fn only_a_boolean_true_acompletion_takes_the_async_path(
    #[case] value: Json,
    #[case] entry: Entry,
    #[case] keeps_client: bool,
) {
    let fields = with(kwargs("gpt-4o-mini", "openai"), "acompletion", value);
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    expect_ok(completion(&execution, call(&fields)).await);
    let observations = execution.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].entry, entry);
    assert_eq!(observations[0].client, keeps_client);
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn an_extra_body_stream_flag_decides_the_delivery(#[case] asynchronous: bool) {
    let fields = optional_params(
        kwargs("gpt-4o-mini", "openai"),
        r#"{"extra_body":{"stream":true}}"#,
    );
    let call_fields = if asynchronous {
        with(fields, "acompletion", Json::Bool(true))
    } else {
        fields
    };
    let execution = FakeExecution::new(events(frames(), &[]));
    let output = expect_ok(completion(&execution, call(&call_fields)).await);
    assert_eq!(
        execution.observations()[0].params_stream,
        StreamParam::Bool(true)
    );
    let chunks = drain(expect_stream(output)).await;
    assert_eq!(joined(&chunks), "hello");
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn an_extra_body_stream_flag_replays_a_chat_answer(#[case] asynchronous: bool) {
    let fields = optional_params(
        kwargs("gpt-4o-mini", "openai"),
        r#"{"extra_body":{"stream":true}}"#,
    );
    let call_fields = if asynchronous {
        with(fields, "acompletion", Json::Bool(true))
    } else {
        fields
    };
    let execution = FakeExecution::new(Answer::Chat(chat_response()));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&call_fields)).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
}

#[tokio::test]
async fn the_sdk_stream_key_is_read_through_the_litellm_params() {
    let params: Json = serde_json::from_str(r#"{"stream":true}"#).unwrap();
    let fields = with(kwargs("gpt-4o-mini", "openai"), "litellm_params", params);
    let execution = FakeExecution::new(events(frames(), &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&fields)).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
}

#[tokio::test]
async fn the_outer_stream_flag_only_adds_a_missing_optional_key() {
    let injected = FakeExecution::new(events(frames(), &[]));
    let output = expect_ok(completion(&injected, call(&streaming())).await);
    assert_eq!(
        injected.observations()[0].params_stream,
        StreamParam::Bool(true)
    );
    let chunks = drain(expect_stream(output)).await;
    assert_eq!(joined(&chunks), "hello");

    let fields = optional_params(streaming(), r#"{"stream":false}"#);
    let retained = FakeExecution::new(events(frames(), &[]));
    let retained_output = expect_ok(completion(&retained, call(&fields)).await);
    // the caller's own stream option reaches the provider as false, and the injection never overwrote it
    assert_eq!(
        retained.observations()[0].params_stream,
        StreamParam::Bool(false)
    );
    assert!(matches!(retained_output, BridgeOutput::Response(_)));

    let text_stream = with(
        kwargs("gpt-4o-mini", "openai"),
        "stream",
        Json::Text(String::from("true")),
    );
    let not_injected = FakeExecution::new(Answer::Chat(chat_response()));
    expect_ok(completion(&not_injected, call(&text_stream)).await);
    assert_eq!(
        not_injected.observations()[0].params_stream,
        StreamParam::Absent
    );
}

#[test]
fn the_stream_flag_follows_the_optional_value_then_the_sdk() {
    let no_optional_params = IndexMap::new();
    assert!(resolve_stream_flag(
        &optional_stream(Some(Json::Bool(true))),
        &sdk_stream(Value::Bool(false))
    ));
    assert!(!resolve_stream_flag(
        &optional_stream(Some(Json::Bool(false))),
        &sdk_stream(Value::Bool(true))
    ));
    assert!(resolve_stream_flag(
        &optional_stream(Some(Json::Null)),
        &sdk_stream(Value::Bool(true))
    ));
    assert!(resolve_stream_flag(
        &no_optional_params,
        &sdk_stream(Value::Bool(true))
    ));
    assert!(!resolve_stream_flag(
        &no_optional_params,
        &sdk_stream(Value::Bool(false))
    ));
    assert!(resolve_stream_flag(
        &no_optional_params,
        &sdk_stream(Value::from("yes"))
    ));
}

#[rstest]
#[case::empty_text(Json::Text(String::new()), false)]
#[case::text(Json::Text(String::from("x")), true)]
#[case::one(Json::Number(serde_json::Number::from(1_i32)), true)]
#[case::zero(Json::Number(serde_json::Number::from(0_i32)), false)]
#[case::list_with_false(Json::List(vec![Json::Bool(false)]), true)]
#[case::empty_list(Json::List(Vec::new()), false)]
#[case::object(Json::Object(IndexMap::from([(String::from("a"), Json::Bool(true))])), true)]
#[case::empty_object(Json::Object(IndexMap::new()), false)]
fn python_truthiness_decides_the_stream_flag(#[case] value: Json, #[case] expected: bool) {
    let sdk = sdk_stream(Value::Bool(true));
    assert_eq!(
        resolve_stream_flag(&optional_stream(Some(value)), &sdk),
        expected
    );
}

#[test]
fn json_mode_is_read_only_as_a_boolean() {
    assert_eq!(
        json_mode(&with(IndexMap::new(), "json_mode", Json::Bool(true))),
        Some(true)
    );
    assert_eq!(
        json_mode(&with(IndexMap::new(), "json_mode", Json::Bool(false))),
        Some(false)
    );
    assert_eq!(
        json_mode(&with(IndexMap::new(), "json_mode", Json::Null)),
        None
    );
    assert_eq!(
        json_mode(&with(
            IndexMap::new(),
            "json_mode",
            Json::Text(String::from("true"))
        )),
        None
    );
    assert_eq!(json_mode(&IndexMap::new()), None);
}

#[tokio::test]
async fn a_collected_stream_becomes_a_chat_response() {
    let fields = kwargs("gpt-4o-mini", "openai");
    let execution = FakeExecution::new(events(frames(), &[("model_id", "hidden-1")]));
    let record = expect_response(expect_ok(completion(&execution, call(&fields)).await));
    assert_eq!(record.id, "resp_a");
    assert_eq!(record.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(record.created, 5);
    assert_eq!(record.choices.len(), 1);
    assert_eq!(record.choices[0].message.content.as_deref(), Some("hello"));
    assert_eq!(record.choices[0].finish_reason, "stop");
    assert_eq!(
        record.hidden_params.get("model_id"),
        Some(&Value::from("hidden-1"))
    );
}

#[tokio::test]
async fn a_collected_response_keeps_the_callers_fingerprint_extras_and_headers() {
    let mut grouped = typed_response("hi");
    grouped.existing_hidden = hidden(&[("model_id", "hidden-1")]);
    grouped.existing_hidden.insert(
        String::from("additional_headers"),
        serde_json::from_str(r#"{"shared":"provider","provider_only":"p"}"#).unwrap(),
    );
    let fields = kwargs("gpt-4o-mini", "openai");
    let mut bridge = call(&fields);
    bridge.model_response = caller_with_identity_extras();
    let execution = FakeExecution::new(Answer::Response(Box::new(grouped)));
    let record = expect_response(expect_ok(completion(&execution, bridge).await));
    assert_eq!(record.system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(record.extra.get("trace"), Some(&Value::from("t-1")));
    assert_eq!(record.created, 5);
    assert_eq!(record.object, "chat.completion");
    let headers = record.hidden_params.get("additional_headers").unwrap();
    assert_eq!(headers.get("shared"), Some(&Value::from("caller")));
    assert_eq!(headers.get("caller_only"), Some(&Value::from("c")));
    assert_eq!(headers.get("provider_only"), Some(&Value::from("p")));
    assert_eq!(
        record.hidden_params.get("model_id"),
        Some(&Value::from("hidden-1"))
    );
}

#[rstest]
#[case::incomplete("response.incomplete")]
#[case::failed("response.failed")]
#[tokio::test]
async fn incomplete_and_failed_events_also_complete_a_stream(#[case] kind: &str) {
    let execution = FakeExecution::new(events(vec![terminal_frame(kind, "hi")], &[]));
    let record = expect_response(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    assert_eq!(record.choices[0].message.content.as_deref(), Some("hi"));
}

#[tokio::test]
async fn a_stream_without_a_completed_response_is_an_error() {
    let fields = kwargs("gpt-4o-mini", "openai");
    let execution = FakeExecution::new(events(vec![created_event()], &[]));
    let text = expect_error(completion(&execution, call(&fields)).await);
    assert_eq!(text, "Stream ended without a completed response");

    let null_response = FakeExecution::new(events(
        vec![sse(r#"{"type":"response.completed","response":null}"#)],
        &[],
    ));
    let null_text = expect_error(completion(&null_response, call(&fields)).await);
    assert_eq!(null_text, "Stream ended without a completed response");
}

#[tokio::test]
async fn a_stream_failure_propagates() {
    let failure = StreamFailure::Reported {
        status_code: 500,
        message: String::from("bad upstream"),
    };
    let execution = FakeExecution::new(Answer::Events(Box::new(FakeEvents::failing(failure))));
    let text = expect_error(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(text, "the responses stream failed");
}

#[tokio::test]
async fn existing_hidden_params_win_over_the_stream_map() {
    let body = r#"{"type":"response.completed","response":{"id":"resp_a","created_at":1700000000,"_hidden_params":{"a":"1","only":"kept"},"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]}}"#;
    let execution = FakeExecution::new(events(vec![sse(body)], &[("a", "2"), ("b", "3")]));
    let record = expect_response(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    assert_eq!(record.hidden_params.get("a"), Some(&Value::from("1")));
    assert_eq!(record.hidden_params.get("only"), Some(&Value::from("kept")));
    assert_eq!(record.hidden_params.get("b"), Some(&Value::from("3")));
}

#[rstest]
#[case::plain(false)]
#[case::streaming(true)]
#[tokio::test]
async fn a_typed_responses_object_ignores_the_stream_flag(#[case] stream_requested: bool) {
    let fields = if stream_requested {
        streaming()
    } else {
        kwargs("gpt-4o-mini", "openai")
    };
    let execution = FakeExecution::new(Answer::Response(Box::new(typed_response("hi"))));
    let record = expect_response(expect_ok(completion(&execution, call(&fields)).await));
    assert_eq!(record.choices[0].message.content.as_deref(), Some("hi"));
}

#[tokio::test]
async fn a_non_object_payload_is_unexpected() {
    let payload = BridgeResponse {
        payload: ResponsePayload::Dict(Json::Text(String::from("hi"))),
        existing_hidden: Map::new(),
        stream_hidden: Map::new(),
    };
    let execution = FakeExecution::new(Answer::Response(Box::new(payload)));
    let text = expect_error(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(text, "Unexpected responses stream payload");
}

#[tokio::test]
async fn an_invalid_dict_field_falls_back_to_lenient_construction() {
    let payload: Json = serde_json::from_str(
        r#"{"id":"resp_a","created_at":"later","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]}"#,
    )
    .unwrap();
    let response = BridgeResponse {
        payload: ResponsePayload::Dict(payload),
        existing_hidden: Map::new(),
        stream_hidden: Map::new(),
    };
    let execution = FakeExecution::new(Answer::Response(Box::new(response)));
    let record = expect_response(expect_ok(
        completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await,
    ));
    assert_eq!(record.choices[0].message.content.as_deref(), Some("hi"));
    assert_eq!(record.id, "resp_a");
    assert_eq!(record.created, 5);
}

#[tokio::test]
async fn a_dict_payload_that_cannot_be_constructed_names_completion() {
    let payload: Json =
        serde_json::from_str(r#"{"id":"resp_a","created_at":1,"output":[5]}"#).unwrap();
    let response = BridgeResponse {
        payload: ResponsePayload::Dict(payload),
        existing_hidden: Map::new(),
        stream_hidden: Map::new(),
    };
    let execution = FakeExecution::new(Answer::Response(Box::new(response)));
    let text = expect_error(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert!(
        text.starts_with("the completed responses payload could not be read"),
        "{text}"
    );
}

#[tokio::test]
async fn a_provider_failure_propagates() {
    let execution = FakeExecution::failing();
    let text = expect_error(completion(&execution, call(&kwargs("gpt-4o-mini", "openai"))).await);
    assert_eq!(text, "the fake execution failed");
}

#[tokio::test]
async fn a_cached_chat_stream_bypasses_translation_and_is_post_processed() {
    let processing = RecordingProcessing::default();
    let deps = post_stream(&processing);
    let fields = streaming();
    let mut bridge = call(&fields);
    bridge.post_stream = Some(&deps);
    let cached = FakeChunks::new(&["{\"id\":\"cached-1\"}"]);
    let execution = FakeExecution::new(Answer::Cached(Box::new(cached)));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, bridge).await,
    )))
    .await;
    assert_eq!(
        chunks,
        vec![serde_json::from_str::<Json>(r#"{"replaced":true}"#).unwrap()]
    );
    assert_eq!(
        processing.calls.lock().unwrap().as_slice(),
        &[ChatConfigTag::OpenAiGpt]
    );
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn a_cached_chat_stream_must_be_consumed_by_a_nonstreaming_caller(
    #[case] asynchronous: bool,
) {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let cached = FakeChunks::tracking(&["{\"id\":\"cached-1\"}", "{\"id\":\"cached-2\"}"], &reads);
    let execution = FakeExecution::new(Answer::Cached(Box::new(cached)));
    let fields = if asynchronous {
        with(
            kwargs("gpt-4o-mini", "openai"),
            "acompletion",
            Json::Bool(true),
        )
    } else {
        kwargs("gpt-4o-mini", "openai")
    };
    let text = expect_error(completion(&execution, call(&fields)).await);
    assert_eq!(text, "Stream ended without a completed response");
    let seen = reads.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[0].as_ref().and_then(id_of),
        Some(String::from("cached-1"))
    );
    assert_eq!(
        seen[1].as_ref().and_then(id_of),
        Some(String::from("cached-2"))
    );
    assert!(seen[2].is_none());
}

#[tokio::test]
async fn a_translated_stream_keeps_one_id_and_one_finish_reason() {
    let execution = FakeExecution::new(events(frames(), &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
    assert_eq!(finishes(&chunks), vec![String::from("stop")]);
    let identifiers = ids(&chunks);
    assert!(!identifiers.is_empty());
    assert!(identifiers.iter().all(|id| id == "resp_a"));
}

#[tokio::test]
async fn a_translated_stream_closes_when_the_events_end_without_a_terminal() {
    let lines = vec![created_event(), delta_event("hi")];
    let execution = FakeExecution::new(events(lines, &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hi");
    assert_eq!(finishes(&chunks), vec![String::from("stop")]);
}

#[tokio::test]
async fn the_translator_and_wrapper_stamp_chunks_from_the_injected_sources() {
    let execution = FakeExecution::new(events(vec![delta_event("hi")], &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&streaming())).await,
    )))
    .await;
    let expected_id = format!("chatcmpl-{FIXED_UUID}");
    let identifiers = ids(&chunks);
    assert!(!identifiers.is_empty());
    assert!(identifiers.iter().all(|id| id == &expected_id));
    assert_eq!(
        created_of(&chunks[0]),
        Some(Json::Number(serde_json::Number::from(FIXED_CLOCK)))
    );
    assert_eq!(
        hidden_member_of(&chunks[0], "created_at"),
        Some(Json::Number(
            serde_json::Number::from_f64(FIXED_TIMESTAMP).unwrap()
        ))
    );
}

#[tokio::test]
async fn a_translation_failure_reaches_the_bridge_caller_typed() {
    let execution = FakeExecution::new(events(vec![sse("{not json")], &[]));
    let mut stream = expect_stream(expect_ok(completion(&execution, call(&streaming())).await));
    let error = stream.next().await.unwrap_err();
    let BridgeStreamError::Translate(error) = error else {
        panic!("a malformed event is a translation failure");
    };
    assert!(
        error
            .to_string()
            .contains("Expecting property name enclosed in double quotes"),
        "{error}"
    );
}

#[tokio::test]
async fn post_stream_processing_selects_the_config_with_the_unprefixed_model() {
    let processing = RecordingProcessing::default();
    let deps = post_stream_named(&processing);
    let fields = with(kwargs("o1", "openai"), "stream", Json::Bool(true));
    let mut bridge = call(&fields);
    bridge.post_stream = Some(&deps);
    let execution = FakeExecution::new(events(frames(), &[]));
    expect_ok(completion(&execution, bridge).await);
    assert_eq!(execution.observations()[0].model, "openai/o1");
    assert_eq!(
        processing.calls.lock().unwrap().as_slice(),
        &[ChatConfigTag::OpenAiOSeries]
    );
}

#[tokio::test]
async fn an_unlisted_provider_leaves_the_stream_unchanged() {
    let processing = RecordingProcessing::default();
    let deps = post_stream(&processing);
    let fields = with(
        kwargs("gpt-4o-mini", "synthetic"),
        "stream",
        Json::Bool(true),
    );
    let mut bridge = call(&fields);
    bridge.post_stream = Some(&deps);
    let execution = FakeExecution::new(events(frames(), &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, bridge).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
    assert!(processing.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_provider_leaves_the_stream_unchanged() {
    let processing = RecordingProcessing::default();
    let deps = post_stream(&processing);
    let fields = with(
        kwargs("gpt-4o-mini", "not-a-provider"),
        "stream",
        Json::Bool(true),
    );
    let mut bridge = call(&fields);
    bridge.post_stream = Some(&deps);
    let execution = FakeExecution::new(events(frames(), &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, bridge).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
    assert!(processing.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_json_record_leaves_the_stream_unchanged() {
    let processing = RecordingProcessing::default();
    let deps = post_stream_missing_record(&processing);
    let fields = with(
        kwargs("gpt-4o-mini", "synthetic"),
        "stream",
        Json::Bool(true),
    );
    let mut bridge = call(&fields);
    bridge.post_stream = Some(&deps);
    let execution = FakeExecution::new(events(frames(), &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, bridge).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello");
    assert!(processing.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dropping_the_translated_stream_drops_the_event_stream() {
    let flag = Arc::new(Mutex::new(false));
    let events =
        FakeEvents::new(vec![created_event()], Map::new()).with_drop_flag(Arc::clone(&flag));
    let stream = TranslatedStream::new(
        Box::new(events),
        &StreamConfig {
            provider: "openai",
            model: "gpt-4o-mini",
            json_mode: Some(true),
            limits: ApiLimits::default(),
            stamps: stamps(),
        },
    );
    drop(stream);
    assert!(*flag.lock().unwrap());
}

#[tokio::test]
async fn identical_chunks_stop_at_the_repeated_chunk_limit() {
    let fields = streaming();
    let at_limit: Vec<Vec<u8>> = iter::once(created_event())
        .chain(iter::repeat_with(|| delta_event("hello")).take(100))
        .chain(iter::once(sse(&terminal_event("hello"))))
        .collect();
    let execution = FakeExecution::new(events(at_limit, &[]));
    let chunks = drain(expect_stream(expect_ok(
        completion(&execution, call(&fields)).await,
    )))
    .await;
    assert_eq!(joined(&chunks), "hello".repeat(100));

    let past_limit: Vec<Vec<u8>> = iter::once(created_event())
        .chain(iter::repeat_with(|| delta_event("hello")).take(101))
        .chain(iter::once(sse(&terminal_event("hello"))))
        .collect();
    let over_limit = FakeExecution::new(events(past_limit, &[]));
    let mut stream = expect_stream(expect_ok(completion(&over_limit, call(&fields)).await));
    loop {
        match stream.next().await {
            Ok(Some(_)) => {}
            Ok(None) => panic!("the repetition limit never fired"),
            Err(error) => {
                assert!(matches!(error, BridgeStreamError::Wrapper(_)));
                break;
            }
        }
    }
}
