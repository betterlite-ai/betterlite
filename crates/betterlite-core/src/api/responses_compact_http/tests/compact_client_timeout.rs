use std::time::Duration;

use betterlite_models::decisions::Json as LoggedJson;

use super::support::{
    COMPACT_OK, Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source,
    TestCall, Wire, capture_debug, compact_deps, openai_config, optional_params, params,
    slow_server, text_input,
};
use crate::api::responses_compact_http::config::CompactParams;
use crate::api::responses_compact_http::error::CompactError;
use crate::api::responses_compact_http::service::facts::{Sent, timeout_message};
use crate::api::responses_compact_http::service::{
    ClientKind, CompactDeps, SharedSession, SuppliedClient,
};

const SYNC_EVENTS: [&str; 8] = [
    "validate",
    "url",
    "transform",
    "normalize",
    "sign",
    "log",
    "send",
    "response",
];

fn logged_pre_call(fixture: &Fixture) {
    assert_eq!(
        fixture.logged("api_base"),
        Some(LoggedJson::Text(String::from(
            "http://mock.test/v1/responses/compact"
        )))
    );
    assert_eq!(
        fixture.input(),
        Some(LoggedJson::Text(String::from("hello")))
    );
}

pub(super) async fn supplied_sync_client() {
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
    logged_pre_call(&fixture);
    assert_eq!(config.models(), vec![String::from("gpt-4o")]);
    assert_eq!(events.recorded(), SYNC_EVENTS);
}

pub(super) async fn wrong_kind_client() {
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
        capture
            .logs
            .take()
            .iter()
            .any(|line| line
                == "Creating HTTP client for compact_response with shared_session: None")
    );
}

pub(super) async fn absent_client() {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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
            .any(|line| line == "Creating HTTP client for compact_response with shared_session: 7")
    );
}

pub(super) async fn timeouts_forwarded() {
    let events = Events::new();
    let config = openai_config("sk-unused", &events);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
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

pub(super) fn timeout_messages() {
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
}

pub(super) fn async_timeout_messages() {
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
}

pub(super) async fn sync_timeout() {
    let serving = slow_server();
    let events = Events::new();
    let config = RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
    let source = ProxiedSource::new(IMPATIENT);
    let deps = compact_deps(&config, &source);
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

pub(super) async fn configured_sync_timeout() {
    let serving = slow_server();
    let events = Events::new();
    let config = RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
    let source = ProxiedSource::new(IMPATIENT);
    let deps = compact_deps(&config, &source);
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

pub(super) async fn async_timeout() {
    let serving = slow_server();
    let events = Events::new();
    let config = RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
    let source = ProxiedSource::new(IMPATIENT);
    let deps = compact_deps(&config, &source);
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
