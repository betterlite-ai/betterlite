#![cfg(test)]

mod compact_client_timeout;
mod compact_signed_body_order;
mod compact_transport_errors;
mod support;

use std::time::Duration;

use betterlite_models::decisions::Json as LoggedJson;
use betterlite_openai::response_events::ReplyBody;
use http::Method;
use rstest::rstest;

use self::support::{
    COMPACT_OK, Events, Fixture, Reply, Source, TestCall, Wire, body_json, caller_headers,
    compact_deps, extra_body, json, keyed_params, openai_config, optional_params, text_input,
};
use crate::api::responses_compact_http::service::ClientKind;

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

#[tokio::test]
async fn compact_transport_errors() {
    let params = keyed_params("http://mock.test/v1", "sk-provider");
    let input = text_input("hello");
    let optional = optional_params("{}");
    let call = TestCall::new("gpt-4o", &input, &optional, &params);
    compact_transport_errors::status_429(&call).await;
    compact_transport_errors::status_304(&call).await;
    compact_transport_errors::malformed_200(&call).await;
    compact_transport_errors::non_utf8_200(&call).await;
    compact_transport_errors::non_utf8_429(&call).await;
    compact_transport_errors::sign_failure(&call).await;
    compact_transport_errors::validation_failure(&call).await;
    compact_transport_errors::normalization_failure(&call).await;
    compact_transport_errors::base_url_failure(&call).await;
    compact_transport_errors::transformed_url_failure(&call).await;
    compact_transport_errors::header_name_failure(&call).await;
    compact_transport_errors::header_value_failure(&call).await;
    compact_transport_errors::transport_failure(&call).await;
}

#[tokio::test]
async fn compact_client_timeout() {
    compact_client_timeout::supplied_sync_client().await;
    compact_client_timeout::wrong_kind_client().await;
    compact_client_timeout::absent_client().await;
    compact_client_timeout::timeouts_forwarded().await;
    compact_client_timeout::timeout_messages();
    compact_client_timeout::async_timeout_messages();
    compact_client_timeout::sync_timeout().await;
    compact_client_timeout::configured_sync_timeout().await;
    compact_client_timeout::async_timeout().await;
}
