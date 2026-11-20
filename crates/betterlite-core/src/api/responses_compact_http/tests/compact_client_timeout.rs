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
use crate::api::vector_store_http_write::error::BaseLlmException;

const SYNC_NONE: &str = "litellm.Timeout: Connection timed out after None seconds.";
const SYNC_SEVEN: &str = "litellm.Timeout: Connection timed out after 7.0 seconds.";
const SYNC_FRACTION: &str = "litellm.Timeout: Connection timed out after 0.15 seconds.";
const ASYNC_PREFIX: &str = "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=";
const ASYNC_DEFAULT: &str =
    "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=1.234 seconds";
const ASYNC_FRACTION: &str =
    "litellm.Timeout: Connection timed out. Timeout passed=0.15, time taken=0.5 seconds";
const ASYNC_ZERO: &str =
    "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=0.0 seconds";

const SYNC_BASE: Sent = Sent {
    mode: ClientKind::Sync,
    requested: None,
    effective: Duration::from_millis(100),
    elapsed: Duration::from_millis(50),
};

const ASYNC_BASE: Sent = Sent {
    mode: ClientKind::Asynchronous,
    requested: None,
    effective: Duration::from_millis(100),
    elapsed: Duration::from_millis(1234),
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

type Calls = Vec<(Option<bool>, Option<SharedSession>)>;

struct Selected {
    supplied: bool,
    params: CompactParams,
    shared_session: Option<SharedSession>,
    calls: Calls,
    debug: &'static str,
}

fn selection_rows() -> [Selected; 2] {
    [
        Selected {
            supplied: true,
            params: params("http://mock.test/v1"),
            shared_session: None,
            calls: vec![(None, None)],
            debug: "Creating HTTP client for compact_response with shared_session: None",
        },
        Selected {
            supplied: false,
            params: CompactParams {
                api_base: Some(String::from("http://mock.test/v1")),
                api_key: None,
                ssl_verify: Some(false),
            },
            shared_session: Some(SharedSession(7)),
            calls: vec![(Some(false), Some(SharedSession(7)))],
            debug: "Creating HTTP client for compact_response with shared_session: 7",
        },
    ]
}

pub(super) async fn client_selection() {
    for row in selection_rows() {
        let events = Events::new();
        let config = openai_config("sk-unused", &events);
        let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
        let source = Source::new(wire.clone());
        let supplied = SuppliedClient {
            kind: ClientKind::Sync,
            transport: &wire,
        };
        let deps = if row.supplied {
            CompactDeps {
                config: &config,
                source: &source,
                supplied: Some(supplied),
            }
        } else {
            compact_deps(&config, &source)
        };
        let input = text_input("hello");
        let optional = optional_params("{}");
        let mut call = TestCall::new("gpt-4o", &input, &optional, &row.params);
        call.shared_session = row.shared_session;
        let mut fixture = Fixture::new("responses");
        let capture = capture_debug();
        fixture
            .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
            .await
            .expect("the async handler picks its client");
        assert_eq!(source.calls(), row.calls);
        assert!(capture.logs.take().iter().any(|line| line == row.debug));
    }
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
    let rows = [
        (SYNC_BASE, SYNC_NONE),
        (
            Sent {
                requested: Some(Duration::from_secs(7)),
                ..SYNC_BASE
            },
            SYNC_SEVEN,
        ),
        (
            Sent {
                requested: Some(Duration::from_millis(150)),
                ..SYNC_BASE
            },
            SYNC_FRACTION,
        ),
        (ASYNC_BASE, ASYNC_DEFAULT),
        (
            Sent {
                effective: Duration::from_millis(150),
                elapsed: Duration::from_millis(500),
                ..ASYNC_BASE
            },
            ASYNC_FRACTION,
        ),
        (
            Sent {
                elapsed: Duration::ZERO,
                ..ASYNC_BASE
            },
            ASYNC_ZERO,
        ),
    ];
    for (sent, expected) in rows {
        assert_eq!(timeout_message(&sent), expected);
    }
}

enum Timed {
    Exact(&'static str),
    AsyncPrefix,
}

pub(super) async fn slow_timeouts() {
    let rows = [
        (ClientKind::Sync, None, Timed::Exact(SYNC_NONE)),
        (
            ClientKind::Sync,
            Some(Duration::from_millis(150)),
            Timed::Exact(SYNC_FRACTION),
        ),
        (ClientKind::Asynchronous, None, Timed::AsyncPrefix),
    ];
    let params = params("http://unused.test");
    let input = text_input("hello");
    let optional = optional_params("{}");
    let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
    for (mode, timeout, expected) in rows {
        let serving = slow_server();
        let events = Events::new();
        let config =
            RecordingConfig::new(&format!("http://{}/v1/responses", serving.address), &events);
        let source = ProxiedSource::new(IMPATIENT);
        let deps = compact_deps(&config, &source);
        call.timeout = timeout;
        let mut fixture = Fixture::new("responses");
        let result = fixture.run(&deps, mode, &call.call(), &events).await;
        let Err(CompactError::Provider(error)) = result else {
            panic!("a timeout is a provider error");
        };
        check_timeout(&error, expected);
    }
}

fn check_timeout(error: &BaseLlmException, expected: Timed) {
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert!(error.headers.is_empty());
    match expected {
        Timed::Exact(text) => assert_eq!(error.message, text),
        Timed::AsyncPrefix => {
            assert!(error.message.starts_with(ASYNC_PREFIX));
            assert!(error.message.ends_with(" seconds"));
            let elapsed = error
                .message
                .trim_start_matches(ASYNC_PREFIX)
                .trim_end_matches(" seconds")
                .parse::<f64>()
                .expect("the elapsed time parses");
            assert!(elapsed > 0.0_f64);
        }
    }
}
