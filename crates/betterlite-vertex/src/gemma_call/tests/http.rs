#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{
    ENDPOINT, Event, Fixture, MODEL, Reply, STRIPPED_MODEL, TOKEN, body_json, run,
};
use crate::gemma_call::plan::{SuppliedClientKind, for_call};
use crate::gemma_envelope::response::GemmaReply;

const VALID_REPLY: &str = r#"{"predictions":{"id":"chatcmpl-provider","object":"chat.completion","created":1730000000,"model":"provider-gemma","choices":[{"index":0,"message":{"role":"assistant","content":"hello there"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}}"#;
const PREDICT_URL: &str = "https://ep.example/custom:predict";

#[test]
fn single_nonstreaming_post() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        call,
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
    let debug = format!("{:?}", planned);
    assert!(debug.contains("Authorization"));
    assert!(!debug.contains(TOKEN));
    let run = run(
        call,
        Reply::Status(201, VALID_REPLY),
        Some(SuppliedClientKind::AsyncHandler),
    );
    let headers = &run.caller_requests[0].headers;
    assert_eq!(headers["Authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(headers["Content-Type"], "application/json");
    let Err(error) = run.result else {
        panic!("a 201 is rejected");
    };
    assert!(!format!("{:?}", error).contains(TOKEN));
    assert!(!error.to_string().contains(TOKEN));
}

#[test]
fn synthetic_stream_order() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), true);
    let run = run(
        call,
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
        call,
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
fn upstream_201_rejection() {
    let fixture = Fixture::new();
    let call = fixture.call(MODEL, Some(ENDPOINT), false);
    let run = run(
        call,
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
