use betterlite_http::proxy::transport_error::TransportError;

use super::support::{
    COMPACT_OK, Events, FailureStage, Fixture, RecordingConfig, Reply, Script, Source, TestCall,
    Wire, caller_headers, compact_deps, masked_headers, openai_config,
};
use crate::api::responses_compact_http::error::{CompactError, Stage};
use crate::api::responses_compact_http::service::ClientKind;
use crate::api::vector_store_http_write::error::HeaderMap;

const STATUS_304_EVENTS: [&str; 7] = [
    "validate",
    "url",
    "transform",
    "normalize",
    "sign",
    "log",
    "send",
];

pub(super) async fn status_429(call: &TestCall<'_>) {
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
    let deps = compact_deps(&config, &source);
    let mut fixture = Fixture::new("responses");
    let result = fixture
        .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
        .await;
    let Err(CompactError::Provider(error)) = result else {
        panic!("a 429 is a provider error");
    };
    let masked = masked_headers(&[
        ("content-length", "9"),
        ("content-type", "text/plain"),
        ("retry-after", "3"),
    ]);
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.headers, masked);
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        config.errors(),
        vec![(String::from("slow down"), 429_u16, masked)]
    );
}

pub(super) async fn status_304(call: &TestCall<'_>) {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let wire = Wire::replying(
        Reply::json(304, "")
            .with_header("content-type", "text/plain")
            .with_header("etag", "\"v1\"")
            .with_header("content-encoding", "gzip")
            .with_header("content-length", "999"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let mut fixture = Fixture::new("responses");
    let result = fixture
        .run(&deps, ClientKind::Sync, &call.call(), &events)
        .await;
    let Err(CompactError::Provider(error)) = result else {
        panic!("a 304 is a provider error, as raise_for_status raises for it");
    };
    let kept = masked_headers(&[("content-type", "text/plain"), ("etag", "\"v1\"")]);
    assert_eq!(error.status_code, 304);
    assert!(!error.status_code_is_synthesized);
    assert!(error.message.is_empty());
    assert_eq!(error.headers, kept);
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(events.recorded(), STATUS_304_EVENTS);
    assert_eq!(config.errors(), vec![(String::new(), 304_u16, kept)]);
}

pub(super) async fn malformed_200(call: &TestCall<'_>) {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let wire = Wire::replying(
        Reply::json(200, "{not json").with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn non_utf8_200(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    let wire = Wire::replying(Reply::bytes(200, vec![0xff_u8]), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn non_utf8_429(call: &TestCall<'_>) {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let wire = Wire::replying(
        Reply::bytes(429, vec![0xff_u8, 0xfe_u8]).with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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
        error.headers,
        masked_headers(&[("content-length", "2"), ("retry-after", "3")])
    );
    assert_eq!(wire.requests().len(), 1);
}

pub(super) async fn sign_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
        .scripted(Script::Fail(FailureStage::Sign));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn validation_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
        .scripted(Script::Fail(FailureStage::Validate));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn normalization_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
        .scripted(Script::Fail(FailureStage::Normalize));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn base_url_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("not a url", &events);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn transformed_url_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events)
        .scripted(Script::TransformTo(String::from("not a url")));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) async fn header_name_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let bad = caller_headers(&[("bad name", "value")]);
    let mut bad_call = TestCall::new(call.model, call.input, call.optional_params, call.params);
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

pub(super) async fn header_value_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let bad = caller_headers(&[("X-Trace", "trace\n")]);
    let mut bad_call = TestCall::new(call.model, call.input, call.optional_params, call.params);
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

pub(super) async fn transport_failure(call: &TestCall<'_>) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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
