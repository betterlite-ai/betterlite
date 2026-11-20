#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json as LoggedJson;
use betterlite_openai::response_events::ReplyBody;
use betterlite_prompts::arguments::json::Json as PromptJson;
use http::Method;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use self::support::{
    Events, FailureStage, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Script,
    Source, TestCall, Wire, body_bytes, body_json, caller_headers, capture_debug, extra_body,
    items_input, keyed_params, openai_config, optional_params, params, text_input,
};
use crate::api::responses_compact_http::config::CompactParams;
use crate::api::responses_compact_http::error::{CompactError, Stage};
use crate::api::responses_compact_http::service::facts::{Sent, timeout_message};
use crate::api::responses_compact_http::service::{
    ClientKind, CompactDeps, SharedSession, SuppliedClient,
};
use crate::api::vector_store_http_write::error::HeaderMap;

const COMPACT_OK: &str = r#"{"id":"resp_compact","created_at":100,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;
const ITEM_INPUT: &str =
    r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]"#;
const NORMALIZED_BODY: &str = r#"{"instructions":"short","model":"gpt-4o","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}],"normalized":true}"#;

fn json(text: &str) -> PromptJson {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn logged(text: &str) -> LoggedJson {
    serde_json::from_str(text).expect("the expected log parses")
}

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
    let deps = CompactDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
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
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    config.queue_signed(None);
    config.queue_signed(Some(b"signed".to_vec()));
    config.queue_signed(Some(Vec::new()));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = CompactDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = keyed_params("http://mock.test/v1", "sk-key");
    let input = items_input(ITEM_INPUT);
    let optional = optional_params(r#"{"instructions":"short"}"#);
    let headers = caller_headers(&[("X-Trace", "trace-a")]);
    let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
    call.extra_headers = Some(&headers);
    call.timeout = Some(Duration::from_secs(7));
    let mut fixture = Fixture::new("responses");
    let reply = fixture
        .run(&deps, ClientKind::Sync, &call.call(), &events)
        .await
        .expect("the signed compact call succeeds");
    assert!(matches!(reply.response, ReplyBody::Typed(_)));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        let first = &requests[0];
        assert_eq!(body_json(first), json(NORMALIZED_BODY));
        assert_eq!(first.url.as_str(), "http://mock.test/v1/responses/compact");
        assert_eq!(
            first
                .headers
                .get("x-signed")
                .and_then(|value| value.to_str().ok()),
            Some("yes")
        );
    }
    let signs = config.signs();
    assert_eq!(signs.len(), 1);
    let recorded = &signs[0];
    assert_eq!(recorded.api_base, "http://mock.test/v1/responses/compact");
    assert_eq!(recorded.model.as_deref(), Some("gpt-4o"));
    assert_eq!(recorded.api_key.as_deref(), Some("sk-key"));
    assert_eq!(
        recorded.optional_params.api_base.as_deref(),
        Some("http://mock.test/v1")
    );
    assert_eq!(
        recorded
            .optional_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned())
            .as_deref(),
        Some("sk-key")
    );
    assert_eq!(recorded.optional_params.ssl_verify, None);
    let PromptJson::Object(expected_data) = json(NORMALIZED_BODY) else {
        panic!("the normalized body is an object");
    };
    assert_eq!(recorded.data, expected_data);
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&logged(NORMALIZED_BODY))
    );
    let Some(LoggedJson::Object(logged_headers)) = fixture.logged("headers") else {
        panic!("the pre-call logs the signed headers");
    };
    assert_eq!(
        logged_headers.get("X-Signed"),
        Some(&LoggedJson::Text(String::from("yes")))
    );
    assert_eq!(
        logged_headers.get("X-Trace"),
        Some(&LoggedJson::Text(String::from("trace-a")))
    );
    assert_eq!(
        logged_headers.get("Authorization"),
        Some(&LoggedJson::Text(String::from("Be****er")))
    );
    assert_eq!(
        events.recorded(),
        vec![
            "validate",
            "url",
            "transform",
            "normalize",
            "sign",
            "log",
            "send",
            "response"
        ]
    );
    let second = fixture
        .run(&deps, ClientKind::Sync, &call.call(), &events)
        .await
        .expect("the signed-byte call succeeds");
    assert!(matches!(second.response, ReplyBody::Typed(_)));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(body_bytes(&requests[1]), Some(b"signed".as_slice()));
        assert!(matches!(
            &requests[1].body,
            RequestBody::Bytes { content_type, data }
                if data == b"signed"
                    && content_type.to_str().ok() == Some("application/octet-stream")
        ));
    }
    fixture
        .run(&deps, ClientKind::Sync, &call.call(), &events)
        .await
        .expect("the empty signed-byte call succeeds");
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 3);
        assert!(matches!(
            &requests[2].body,
            RequestBody::Bytes { data, .. } if data.is_empty()
        ));
    }
    let events_async = Events::new();
    let config_async = RecordingConfig::new("http://mock.test/v1/responses", &events_async);
    config_async.queue_signed(None);
    let wire_async = Wire::replying(Reply::json(200, COMPACT_OK), &events_async);
    let source_async = Source::new(wire_async.clone());
    let deps_async = CompactDeps {
        config: &config_async,
        source: &source_async,
        supplied: None,
    };
    let mut fixture_async = Fixture::new("responses");
    fixture_async
        .run(
            &deps_async,
            ClientKind::Asynchronous,
            &call.call(),
            &events_async,
        )
        .await
        .expect("the async compact call signs off the loop");
    assert_eq!(
        events_async.recorded(),
        vec![
            "validate",
            "url",
            "transform",
            "normalize",
            "off_loop",
            "sign",
            "log",
            "send",
            "response"
        ]
    );
    assert!(!events.recorded().contains(&String::from("off_loop")));
}

#[tokio::test]
async fn compact_transport_errors() {
    let params = keyed_params("http://mock.test/v1", "sk-provider");
    let input = text_input("hello");
    let optional = optional_params("{}");
    let call = TestCall::new("gpt-4o", &input, &optional, &params);
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(
            Reply::json(429, "slow down")
                .with_header("retry-after", "3")
                .with_header("content-type", "text/plain")
                .with_header("content-encoding", "gzip")
                .with_header("content-length", "999"),
            &events,
        );
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a 429 is a provider error");
        };
        assert_eq!(error.status_code, 429);
        assert!(!error.status_code_is_synthesized);
        assert_eq!(error.message, "slow down");
        let masked = HeaderMap::from([
            (
                String::from("content-length"),
                LoggedJson::Text(String::from("9")),
            ),
            (
                String::from("content-type"),
                LoggedJson::Text(String::from("text/plain")),
            ),
            (
                String::from("retry-after"),
                LoggedJson::Text(String::from("3")),
            ),
        ]);
        assert_eq!(error.headers, masked);
        assert_eq!(wire.requests().len(), 1);
        assert_eq!(
            config.errors(),
            vec![(String::from("slow down"), 429_u16, masked)]
        );
    }
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(
            Reply::json(200, "{not json").with_header("retry-after", "3"),
            &events,
        );
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a malformed reply fails in the response transform");
        };
        assert_eq!(stage, Stage::Response);
        assert_eq!(failure.to_string(), "{not json");
        let error = CompactError::Step {
            stage,
            source: failure,
        };
        assert_eq!(error.to_string(), "response transform failed: {not json");
        assert_eq!(wire.requests().len(), 1);
        assert!(config.errors().is_empty());
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
        let wire = Wire::replying(Reply::bytes(200, vec![0xff_u8]), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a reply body that is not UTF-8 fails before the response transform");
        };
        assert_eq!(stage, Stage::Response);
        assert!(failure.to_string().starts_with("invalid utf-8 sequence"));
        assert_eq!(wire.requests().len(), 1);
        assert!(config.errors().is_empty());
    }
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(
            Reply::bytes(429, vec![0xff_u8, 0xfe_u8]).with_header("retry-after", "3"),
            &events,
        );
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a 429 with a binary body is a provider error");
        };
        assert_eq!(error.status_code, 429);
        assert!(!error.status_code_is_synthesized);
        assert_eq!(error.message, "2 bytes that are not UTF-8");
        assert_eq!(
            error.headers.get("content-length"),
            Some(&LoggedJson::Text(String::from("2")))
        );
        assert_eq!(wire.requests().len(), 1);
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
            .scripted(Script::Fail(FailureStage::Sign));
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a signing failure bypasses _handle_error");
        };
        assert_eq!(stage, Stage::Sign);
        assert_eq!(failure.to_string(), "scripted provider failure");
        assert!(wire.requests().is_empty());
        assert!(config.errors().is_empty());
        assert_eq!(
            events.recorded(),
            vec!["validate", "url", "transform", "normalize", "sign"]
        );
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
            .scripted(Script::Fail(FailureStage::Validate));
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a validation failure bypasses _handle_error");
        };
        assert_eq!(stage, Stage::Validate);
        assert_eq!(failure.to_string(), "scripted provider failure");
        assert!(wire.requests().is_empty());
        assert!(config.errors().is_empty());
        assert_eq!(events.recorded(), vec!["validate"]);
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
            .scripted(Script::Fail(FailureStage::Normalize));
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a normalization failure bypasses _handle_error");
        };
        assert_eq!(stage, Stage::Normalize);
        assert_eq!(failure.to_string(), "scripted provider failure");
        assert!(wire.requests().is_empty());
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("not a url", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Step {
            stage,
            source: failure,
        }) = result
        else {
            panic!("a base URL that is not a URL fails in the transform");
        };
        assert_eq!(stage, Stage::Transform);
        assert!(
            failure
                .to_string()
                .starts_with("the API base is not a valid URL")
        );
        assert!(wire.requests().is_empty());
        assert!(config.errors().is_empty());
        assert_eq!(events.recorded(), vec!["validate", "url", "transform"]);
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
            .scripted(Script::TransformTo(String::from("not a url")));
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::RequestUrl { url, .. }) = result else {
            panic!("a transformed URL that is not a URL fails before the send");
        };
        assert_eq!(url, "not a url");
        assert!(wire.requests().is_empty());
        assert_eq!(
            events.recorded(),
            vec!["validate", "url", "transform", "normalize", "sign", "log"]
        );
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let bad = caller_headers(&[("bad name", "value")]);
        let mut bad_call = TestCall::new("gpt-4o", &input, &optional, &params);
        bad_call.extra_headers = Some(&bad);
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &bad_call.call(), &events)
            .await;
        let Err(CompactError::HeaderName { name, .. }) = result else {
            panic!("a header name HTTP cannot carry fails before the send");
        };
        assert_eq!(name, "bad name");
        assert!(wire.requests().is_empty());
        assert_eq!(
            events.recorded(),
            vec!["validate", "url", "transform", "normalize", "sign", "log"]
        );
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let bad = caller_headers(&[("X-Trace", "café")]);
        let mut bad_call = TestCall::new("gpt-4o", &input, &optional, &params);
        bad_call.extra_headers = Some(&bad);
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &bad_call.call(), &events)
            .await;
        let Err(CompactError::HeaderValue { name, .. }) = result else {
            panic!("a header value HTTP cannot carry fails before the send");
        };
        assert_eq!(name, "X-Trace");
        assert!(wire.requests().is_empty());
    }
    {
        let events = Events::new();
        let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
        let wire = Wire::failing(TransportError::Closed, &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a transport failure is a provider error");
        };
        assert_eq!(error.status_code, 500);
        assert!(error.status_code_is_synthesized);
        assert_eq!(error.message, TransportError::Closed.to_string());
        assert!(error.headers.is_empty());
        assert_eq!(wire.requests().len(), 1);
        assert_eq!(
            config.errors(),
            vec![(
                TransportError::Closed.to_string(),
                500_u16,
                HeaderMap::new()
            )]
        );
    }
}

#[tokio::test]
async fn compact_client_timeout() {
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &wire,
            }),
        };
        let params = params("http://mock.test/v1");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let call = TestCall::new("gpt-4o", &input, &optional, &params);
        let mut fixture = Fixture::new("responses");
        let capture = capture_debug();
        fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await
            .expect("the supplied sync client is reused");
        assert!(source.calls().is_empty());
        assert!(capture.logs.take().is_empty());
        assert_eq!(wire.requests().len(), 1);
        assert_eq!(
            fixture.logged("api_base"),
            Some(&LoggedJson::Text(String::from(
                "http://mock.test/v1/responses/compact"
            )))
        );
        assert_eq!(
            fixture.input(),
            Some(&LoggedJson::Text(String::from("hello")))
        );
        assert_eq!(config.models(), vec![String::from("gpt-4o")]);
        assert_eq!(
            events.recorded(),
            vec![
                "validate",
                "url",
                "transform",
                "normalize",
                "sign",
                "log",
                "send",
                "response"
            ]
        );
    }
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &wire,
            }),
        };
        let params = params("http://mock.test/v1");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let call = TestCall::new("gpt-4o", &input, &optional, &params);
        let mut fixture = Fixture::new("responses");
        let capture = capture_debug();
        fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await
            .expect("a sync client does not serve the async handler");
        assert_eq!(source.calls(), vec![(None, None)]);
        assert!(
            capture.logs.take().iter().any(|line| line
                == "Creating HTTP client for compact_response with shared_session: None")
        );
    }
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let params = CompactParams {
            api_base: Some(String::from("http://mock.test/v1")),
            api_key: None,
            ssl_verify: Some(false),
        };
        let input = text_input("hello");
        let optional = optional_params("{}");
        let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
        call.shared_session = Some(SharedSession(7));
        let mut fixture = Fixture::new("responses");
        let capture = capture_debug();
        fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await
            .expect("the async handler builds its own client");
        assert_eq!(source.calls(), vec![(Some(false), Some(SharedSession(7)))]);
        assert!(
            capture
                .logs
                .take()
                .iter()
                .any(|line| line
                    == "Creating HTTP client for compact_response with shared_session: 7")
        );
    }
    {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let params = params("http://mock.test/v1");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
        let mut fixture = Fixture::new("responses");
        call.timeout = None;
        fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await
            .expect("the null timeout is forwarded");
        call.timeout = Some(Duration::ZERO);
        fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await
            .expect("the zero timeout is forwarded");
        call.timeout = Some(Duration::from_secs(7));
        fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await
            .expect("the seven second timeout is forwarded");
        let requests = wire.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].timeout, None);
        assert_eq!(requests[1].timeout, Some(Duration::ZERO));
        assert_eq!(requests[2].timeout, Some(Duration::from_secs(7)));
    }

    let sync_none = Sent {
        mode: ClientKind::Sync,
        requested: None,
        effective: Duration::from_millis(100),
        elapsed: Duration::from_millis(50),
    };
    assert_eq!(
        timeout_message(&sync_none),
        "litellm.Timeout: Connection timed out after None seconds."
    );
    let sync_seven = Sent {
        requested: Some(Duration::from_secs(7)),
        ..sync_none
    };
    assert_eq!(
        timeout_message(&sync_seven),
        "litellm.Timeout: Connection timed out after 7.0 seconds."
    );
    let sync_fraction = Sent {
        requested: Some(Duration::from_millis(150)),
        ..sync_none
    };
    assert_eq!(
        timeout_message(&sync_fraction),
        "litellm.Timeout: Connection timed out after 0.15 seconds."
    );
    let async_default = Sent {
        mode: ClientKind::Asynchronous,
        requested: None,
        effective: Duration::from_millis(100),
        elapsed: Duration::from_millis(1234),
    };
    assert_eq!(
        timeout_message(&async_default),
        "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=1.234 seconds"
    );
    let async_passed = Sent {
        effective: Duration::from_millis(150),
        elapsed: Duration::from_millis(500),
        ..async_default
    };
    assert_eq!(
        timeout_message(&async_passed),
        "litellm.Timeout: Connection timed out. Timeout passed=0.15, time taken=0.5 seconds"
    );
    let async_zero = Sent {
        elapsed: Duration::ZERO,
        ..async_default
    };
    assert_eq!(
        timeout_message(&async_zero),
        "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=0.0 seconds"
    );

    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    {
        let events = Events::new();
        let config =
            RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
        let source = ProxiedSource::new(IMPATIENT);
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let params = params("http://unused.test");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let call = TestCall::new("gpt-4o", &input, &optional, &params);
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a sync timeout is a provider error");
        };
        assert_eq!(error.status_code, 408);
        assert!(!error.status_code_is_synthesized);
        assert_eq!(
            error.message,
            "litellm.Timeout: Connection timed out after None seconds."
        );
        assert!(error.headers.is_empty());
    }
    {
        let events = Events::new();
        let config =
            RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
        let source = ProxiedSource::new(IMPATIENT);
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let params = params("http://unused.test");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
        call.timeout = Some(Duration::from_millis(150));
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a sync timeout is a provider error");
        };
        assert_eq!(error.status_code, 408);
        assert_eq!(
            error.message,
            "litellm.Timeout: Connection timed out after 0.15 seconds."
        );
    }
    {
        let events = Events::new();
        let config =
            RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
        let source = ProxiedSource::new(IMPATIENT);
        let deps = CompactDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let params = params("http://unused.test");
        let input = text_input("hello");
        let optional = optional_params("{}");
        let call = TestCall::new("gpt-4o", &input, &optional, &params);
        let mut fixture = Fixture::new("responses");
        let result = fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("an async timeout is a provider error");
        };
        assert_eq!(error.status_code, 408);
        assert!(!error.status_code_is_synthesized);
        let prefix = "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=";
        assert!(error.message.starts_with(prefix));
        assert!(error.message.ends_with(" seconds"));
        let elapsed = error
            .message
            .trim_start_matches(prefix)
            .trim_end_matches(" seconds")
            .parse::<f64>()
            .expect("the elapsed time parses");
        assert!(elapsed > 0.0_f64);
        assert!(error.headers.is_empty());
    }
}
