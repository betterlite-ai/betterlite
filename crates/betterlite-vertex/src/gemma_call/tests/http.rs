#![cfg(test)]

use std::time::Duration;

use betterlite_prompts::arguments::json::Json;

use super::support::{
    ENDPOINT, Event, Fixture, MODEL, Reply, STRIPPED_MODEL, TOKEN, body_json, run,
};
use crate::gemma_call::error::GemmaCallError;
use crate::gemma_call::plan::{SuppliedClientKind, for_call};
use crate::gemma_envelope::post::Timeout;
use crate::gemma_envelope::response::GemmaReply;

const VALID_REPLY: &str = r#"{"predictions":{"id":"chatcmpl-provider","object":"chat.completion","created":1730000000,"model":"provider-gemma","choices":[{"index":0,"message":{"role":"assistant","content":"hello there"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}}"#;
const CONTAINER_429: &str =
    r#"{"predictions":{"object":"error","message":"rate limited","code":429}}"#;
const FILE_WITHOUT_FILE: &str = r#"[{"role":"user","content":[{"type":"file"}]}]"#;
const PREDICT_URL: &str = "https://ep.example/custom:predict";

#[test]
fn single_nonstreaming_post() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    assert_eq!(run.caller_requests.len(), 1);
    assert!(run.pooled_requests.is_empty());
    assert_eq!(run.caller_requests[0].url, PREDICT_URL);
    assert_eq!(run.events, [Event::Posted(String::from(PREDICT_URL))]);
    let body = body_json(&run.caller_requests[0]);
    let Some(Json::List(instances)) = body.get("instances") else {
        panic!("the body carries one instance list");
    };
    let [Json::Object(instance)] = instances.as_slice() else {
        panic!("the body carries one instance");
    };
    assert_eq!(
        instance.get("@requestFormat"),
        Some(&Json::Text(String::from("chatCompletions")))
    );
    assert!(!instance.contains_key("model"));
    assert!(!instance.contains_key("stream"));
    assert!(instance.contains_key("messages"));
    let Ok(GemmaReply::Response(response)) = run.result else {
        panic!("a non-streaming call returns the whole response");
    };
    assert_eq!(response.model, STRIPPED_MODEL);
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("hello there")
    );
    assert!(run.wrapped.is_empty());
}

#[test]
fn header_secrecy() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let planned = for_call(&call, None).unwrap();
    let debug = format!("{planned:?}");
    assert!(debug.contains("Authorization"));
    assert!(!debug.contains(TOKEN));
    let run = run(
        &call,
        Reply::Status(201, VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let headers = &run.caller_requests[0].headers;
    assert_eq!(headers["Authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(headers["Content-Type"], "application/json");
    let Err(error) = run.result else {
        panic!("a 201 is rejected");
    };
    assert!(!format!("{error:?}").contains(TOKEN));
    assert!(!error.to_string().contains(TOKEN));
}

#[test]
fn synthetic_stream_order() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), true);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    assert_eq!(
        run.events,
        [
            Event::Posted(String::from("https://ep.example/custom:predict?alt=sse")),
            Event::Wrapped(String::from(STRIPPED_MODEL)),
        ]
    );
    assert_eq!(run.caller_requests.len(), 1);
    assert_eq!(
        run.caller_requests[0].url,
        "https://ep.example/custom:predict?alt=sse"
    );
    let body = body_json(&run.caller_requests[0]);
    let Some(Json::List(instances)) = body.get("instances") else {
        panic!("the body carries one instance list");
    };
    let [Json::Object(instance)] = instances.as_slice() else {
        panic!("the body carries one instance");
    };
    assert!(!instance.contains_key("stream"));
    let Ok(GemmaReply::Stream(stream)) = run.result else {
        panic!("a requested stream is synthesized");
    };
    assert_eq!(stream.len(), 1);
    assert_eq!(stream[0].model, STRIPPED_MODEL);
    assert_eq!(run.wrapped.len(), 1);
    assert_eq!(run.wrapped[0].model, STRIPPED_MODEL);
}

#[test]
fn raw_client_serves_the_call() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncRaw),
    );
    assert_eq!(run.caller_requests.len(), 1);
    assert!(run.pooled_requests.is_empty());
    let Ok(GemmaReply::Response(response)) = run.result else {
        panic!("a raw async client serves a non-streaming call");
    };
    assert_eq!(response.model, STRIPPED_MODEL);
}

#[test]
fn transport_failure_is_a_vertex_500() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Failure,
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a transport failure is an error");
    };
    assert!(matches!(&error, GemmaCallError::Transport { .. }));
    assert_eq!(error.status_code(), 500);
    assert!(run.wrapped.is_empty());
}

#[test]
fn timeout_status_follows_the_caller() {
    let fixture = Fixture::new();
    let handler_call = fixture.call(MODEL, Some(ENDPOINT), false);
    let handler = run(
        &handler_call,
        Reply::Timeout,
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(handler_error) = handler.result else {
        panic!("a handler timeout is an error");
    };
    assert_eq!(handler_error.status_code(), 408);
    assert_eq!(handler.caller_requests.len(), 1);
    assert!(handler.wrapped.is_empty());
    let pooled_call = fixture.call(MODEL, Some(ENDPOINT), false);
    let pooled = run(
        &pooled_call,
        Reply::Timeout,
        Some(SuppliedClientKind::SyncHandler),
    );
    let Err(pooled_error) = pooled.result else {
        panic!("a pooled timeout is an error");
    };
    assert_eq!(pooled_error.status_code(), 408);
    assert_eq!(pooled.pooled_requests.len(), 1);
    let raw_call = fixture.call(MODEL, Some(ENDPOINT), false);
    let raw = run(
        &raw_call,
        Reply::Timeout,
        Some(SuppliedClientKind::AsyncRaw),
    );
    let Err(raw_error) = raw.result else {
        panic!("a raw client timeout is an error");
    };
    assert_eq!(raw_error.status_code(), 500);
    assert_eq!(raw.caller_requests.len(), 1);
    assert!(raw.wrapped.is_empty());
}

#[test]
fn a_connect_timeout_answers_408() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::ConnectTimeout,
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a connect timeout is an error");
    };
    assert_eq!(error.status_code(), 408);
}

#[test]
fn mismatched_client_falls_back_to_the_pooled_client() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::SyncHandler),
    );
    assert!(run.caller_requests.is_empty());
    assert_eq!(run.pooled_requests.len(), 1);
    assert_eq!(run.pooled_requests[0].url, PREDICT_URL);
    let Ok(GemmaReply::Response(response)) = run.result else {
        panic!("the pooled client serves a non-streaming call");
    };
    assert_eq!(response.model, STRIPPED_MODEL);
}

#[test]
fn a_rejected_body_that_is_not_utf8_names_its_length() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Raw(500, &[0xff, 0xfe, 0x00]),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a 500 is rejected");
    };
    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        "Request failed: 3 bytes that are not UTF-8"
    );
}

#[test]
fn upstream_201_rejection() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Status(201, VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a 201 is rejected");
    };
    assert_eq!(error.status_code(), 201);
    assert_eq!(error.to_string(), format!("Request failed: {VALID_REPLY}"));
    assert!(run.wrapped.is_empty());
    assert_eq!(run.events, [Event::Posted(String::from(PREDICT_URL))]);
}

#[test]
fn container_error_keeps_its_status() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), true);
    let run = run(
        &call,
        Reply::Ok(CONTAINER_429),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("an embedded error record is an error even under a streaming call");
    };
    assert_eq!(error.status_code(), 429);
    assert!(matches!(&error, GemmaCallError::Reply(_)));
    assert!(run.wrapped.is_empty());
    assert_eq!(
        run.events,
        [Event::Posted(String::from(
            "https://ep.example/custom:predict?alt=sse"
        ))]
    );
}

#[test]
fn missing_base_fails_before_posting() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, None, false);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a call without api_base fails in the plan");
    };
    assert_eq!(error.status_code(), 400);
    assert!(matches!(&error, GemmaCallError::Plan(_)));
    assert!(run.caller_requests.is_empty());
    assert!(run.pooled_requests.is_empty());
}

#[test]
fn a_file_block_without_a_file_is_a_400() {
    let mut fixture = Fixture::new();
    fixture.messages = serde_json::from_str(FILE_WITHOUT_FILE).unwrap();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        &call,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let Err(error) = run.result else {
        panic!("a request body that cannot be built fails before posting");
    };
    assert_eq!(error.status_code(), 400);
    assert!(matches!(&error, GemmaCallError::Envelope(_)));
    assert!(run.caller_requests.is_empty());
}

#[test]
fn timeout_reaches_the_transport() {
    let fixture = Fixture::new();
    let mut explicit = fixture.call(MODEL, Some(ENDPOINT), false);
    explicit.timeout = Some(Duration::from_secs(5));
    let handler_run = run(
        &explicit,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    assert_eq!(
        handler_run.caller_requests[0].timeout,
        Timeout::Explicit(Some(Duration::from_secs(5)))
    );
    let defaulted = fixture.call(MODEL, Some(ENDPOINT), false);
    let raw_run = run(
        &defaulted,
        Reply::Ok(VALID_REPLY),
        Some(SuppliedClientKind::AsyncRaw),
    );
    assert_eq!(raw_run.caller_requests[0].timeout, Timeout::ClientDefault);
}
