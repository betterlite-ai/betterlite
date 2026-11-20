#![cfg(test)]

mod compact_client_timeout;
mod compact_signed_body_order;
mod compact_transport_errors;
mod support;

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json as LoggedJson;
use betterlite_openai::response_events::ReplyBody;
use http::Method;
use rstest::rstest;

use self::compact_transport_errors::{
    ConfigSpec, Expect, HeaderExpect, LOG_EVENTS, ProviderExpect, RequestUrlExpect, SIGN_EVENTS,
    STATUS_304_EVENTS, Scenario, StepExpect, TRANSFORM_EVENTS, VALIDATE_EVENTS, WireSpec, check,
    run,
};
use self::support::{
    COMPACT_OK, Events, FailureStage, Fixture, Reply, Source, TestCall, Wire, body_json,
    caller_headers, compact_deps, extra_body, json, keyed_params, masked_headers, openai_config,
    optional_params, text_input,
};
use crate::api::responses_compact_http::error::Stage;
use crate::api::responses_compact_http::service::ClientKind;
use crate::api::vector_store_http_write::error::HeaderMap;

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn compact_post_and_headers(#[case] mode: ClientKind) {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let answer = Reply::json(200, COMPACT_OK)
        .with_header("x-request-id", "req-1")
        .with_header("x-ratelimit-remaining-requests", "9");
    let wire = Wire::replying(answer, &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let params = keyed_params("http://mock.test/v1?api-version=v1", "sk-provider");
    let input = text_input("hello");
    let optional = optional_params(r#"{"instructions":"short"}"#);
    let ignored = extra_body(r#"{"model":"ignored"}"#);
    let headers = caller_headers(&[
        ("Authorization", "Bearer sk-caller"),
        ("X-Trace", "trace-a"),
    ]);
    let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
    call.extra_headers = Some(&headers);
    call.extra_body = Some(&ignored);
    call.timeout = Some(Duration::from_secs(7));
    let mut fixture = Fixture::new("responses");
    let reply = fixture
        .run(&deps, mode, &call.call(), &events)
        .await
        .expect("the compact call succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.path(), "/v1/compact");
    assert_eq!(request.url.query(), Some("api-version=v1/responses"));
    assert_eq!(request.timeout, Some(Duration::from_secs(7)));
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-provider")
    );
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        request
            .headers
            .get("x-trace")
            .and_then(|value| value.to_str().ok()),
        Some("trace-a")
    );
    assert_eq!(
        body_json(request),
        json(r#"{"model":"gpt-4o","input":"hello","instructions":"short"}"#)
    );
    let ReplyBody::Typed(response) = reply.response else {
        panic!("the compact reply validates as a typed response");
    };
    assert_eq!(response.id, "resp_compact");
    assert!(response.output.is_empty());
    assert_eq!(response.status.as_deref(), Some("completed"));
    assert_eq!(response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        reply.headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    assert_eq!(
        reply
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
        Some(&LoggedJson::Text(String::from("9")))
    );
    assert_eq!(
        reply.additional_headers.get("llm_provider-x-request-id"),
        Some(&LoggedJson::Text(String::from("req-1")))
    );
    assert_eq!(
        events
            .recorded()
            .iter()
            .filter(|name| name.as_str() == "send")
            .count(),
        1
    );
    assert_eq!(config.models(), vec![String::from("gpt-4o")]);
}

#[tokio::test]
async fn compact_signed_body_order() {
    compact_signed_body_order::sync_calls().await;
    compact_signed_body_order::async_call().await;
}

#[rstest]
#[case::status_429(Scenario {
    config: ConfigSpec::OpenAi("sk-unused"),
    wire: WireSpec::Reply(
        Reply::json(429, "slow down")
            .with_header("retry-after", "3")
            .with_header("content-type", "text/plain")
            .with_header("content-encoding", "gzip")
            .with_header("content-length", "999"),
    ),
    mode: ClientKind::Asynchronous,
    headers: None,
    expect: Expect::Provider(ProviderExpect {
        status: 429,
        synthesized: false,
        message: String::from("slow down"),
        headers: masked_headers(&[
            ("content-length", "9"),
            ("content-type", "text/plain"),
            ("retry-after", "3"),
        ]),
        logged: true,
        events: None,
    }),
})]
#[case::status_304(Scenario {
    config: ConfigSpec::OpenAi("sk-unused"),
    wire: WireSpec::Reply(
        Reply::json(304, "")
            .with_header("content-type", "text/plain")
            .with_header("etag", "\"v1\"")
            .with_header("content-encoding", "gzip")
            .with_header("content-length", "999"),
    ),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Provider(ProviderExpect {
        status: 304,
        synthesized: false,
        message: String::new(),
        headers: masked_headers(&[("content-type", "text/plain"), ("etag", "\"v1\"")]),
        logged: true,
        events: Some(STATUS_304_EVENTS),
    }),
})]
#[case::malformed_200(Scenario {
    config: ConfigSpec::OpenAi("sk-unused"),
    wire: WireSpec::Reply(
        Reply::json(200, "{not json").with_header("retry-after", "3"),
    ),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Response,
        text: String::from("{not json"),
        prefix: false,
        display: Some("response transform failed: {not json"),
        events: None,
        sent: true,
    }),
})]
#[case::non_utf8_200(Scenario {
    config: ConfigSpec::Recording("http://mock.test/v1/responses"),
    wire: WireSpec::Reply(Reply::bytes(200, vec![0xff_u8])),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Response,
        text: String::from("invalid utf-8 sequence"),
        prefix: true,
        display: None,
        events: None,
        sent: true,
    }),
})]
#[case::non_utf8_429(Scenario {
    config: ConfigSpec::OpenAi("sk-unused"),
    wire: WireSpec::Reply(
        Reply::bytes(429, vec![0xff_u8, 0xfe_u8]).with_header("retry-after", "3"),
    ),
    mode: ClientKind::Asynchronous,
    headers: None,
    expect: Expect::Provider(ProviderExpect {
        status: 429,
        synthesized: false,
        message: String::from("2 bytes that are not UTF-8"),
        headers: masked_headers(&[("content-length", "2"), ("retry-after", "3")]),
        logged: false,
        events: None,
    }),
})]
#[case::sign_failure(Scenario {
    config: ConfigSpec::Scripted("http://mock.test/v1/responses", FailureStage::Sign),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Sign,
        text: String::from("scripted provider failure"),
        prefix: false,
        display: None,
        events: Some(SIGN_EVENTS),
        sent: false,
    }),
})]
#[case::validation_failure(Scenario {
    config: ConfigSpec::Scripted("http://mock.test/v1/responses", FailureStage::Validate),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Validate,
        text: String::from("scripted provider failure"),
        prefix: false,
        display: None,
        events: Some(VALIDATE_EVENTS),
        sent: false,
    }),
})]
#[case::normalization_failure(Scenario {
    config: ConfigSpec::Scripted("http://mock.test/v1/responses", FailureStage::Normalize),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Normalize,
        text: String::from("scripted provider failure"),
        prefix: false,
        display: None,
        events: None,
        sent: false,
    }),
})]
#[case::base_url_failure(Scenario {
    config: ConfigSpec::Recording("not a url"),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::Step(StepExpect {
        stage: Stage::Transform,
        text: String::from("the API base is not a valid URL"),
        prefix: true,
        display: None,
        events: Some(TRANSFORM_EVENTS),
        sent: false,
    }),
})]
#[case::transformed_url_failure(Scenario {
    config: ConfigSpec::Transformed("http://mock.test/v1/responses", "not a url"),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: None,
    expect: Expect::RequestUrl(RequestUrlExpect {
        url: String::from("not a url"),
        events: Some(LOG_EVENTS),
    }),
})]
#[case::header_name_failure(Scenario {
    config: ConfigSpec::Recording("http://mock.test/v1/responses"),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: Some(caller_headers(&[("bad name", "value")])),
    expect: Expect::Header(HeaderExpect::Name {
        name: String::from("bad name"),
        events: Some(LOG_EVENTS),
    }),
})]
#[case::header_value_failure(Scenario {
    config: ConfigSpec::Recording("http://mock.test/v1/responses"),
    wire: WireSpec::Reply(Reply::json(200, COMPACT_OK)),
    mode: ClientKind::Sync,
    headers: Some(caller_headers(&[("X-Trace", "trace\n")])),
    expect: Expect::Header(HeaderExpect::Value {
        name: String::from("X-Trace"),
    }),
})]
#[case::transport_failure(Scenario {
    config: ConfigSpec::Recording("http://mock.test/v1/responses"),
    wire: WireSpec::Closed,
    mode: ClientKind::Asynchronous,
    headers: None,
    expect: Expect::Provider(ProviderExpect {
        status: 500,
        synthesized: true,
        message: TransportError::Closed.to_string(),
        headers: HeaderMap::new(),
        logged: true,
        events: None,
    }),
})]
#[tokio::test]
async fn compact_transport_errors(#[case] scenario: Scenario) {
    let params = keyed_params("http://mock.test/v1", "sk-provider");
    let input = text_input("hello");
    let optional = optional_params("{}");
    let call = TestCall::new("gpt-4o", &input, &optional, &params);
    let outcome = run(&scenario, &call).await;
    check(&outcome, &scenario.expect);
}

#[tokio::test]
async fn compact_client_timeout() {
    compact_client_timeout::supplied_sync_client().await;
    compact_client_timeout::client_selection().await;
    compact_client_timeout::timeouts_forwarded().await;
    compact_client_timeout::timeout_messages();
    compact_client_timeout::slow_timeouts().await;
}
