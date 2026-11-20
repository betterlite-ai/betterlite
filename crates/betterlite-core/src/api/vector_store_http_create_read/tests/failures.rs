use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::store::CreateParams;
use http::HeaderValue;
use rstest::rstest;

use super::run;
use super::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire,
    caller_headers, deps, params, target,
};
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{
    CreateCall, CreateReadCall, CreateReadDeps, ListCall, ListQuery, Operation, RetrieveCall,
};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::error::{BaseLlmException, WriteError};
use crate::api::vector_store_http_write::service::ClientKind;

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;

fn call_for<'call>(
    operation: Operation,
    params: &'call WriteParams,
    optional: &'call CreateParams,
) -> CreateReadCall<'call> {
    let target = target(params);
    match operation {
        Operation::Create => CreateReadCall::Create(CreateCall {
            target,
            optional_params: optional,
            extra_body: None,
        }),
        Operation::Retrieve => CreateReadCall::Retrieve(RetrieveCall {
            target,
            vector_store_id: "vs_a",
            extra_body: None,
        }),
        Operation::List => CreateReadCall::List(ListCall {
            target,
            query: ListQuery {
                after: None,
                before: None,
                limit: None,
                order: None,
            },
            extra_body: None,
        }),
    }
}

// A status outside 2xx reaches the POST and the GET as the provider's own error, with the headers the reply carried
fn check_status_failure<'result>(
    result: &'result Result<Json, CreateReadError>,
    config: &RecordingConfig,
) -> &'result BaseLlmException {
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a status of 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text("3".to_owned()))
    );
    assert_eq!(config.errors().len(), 1);
    error
}

fn check_list_failure(result: &Result<Json, CreateReadError>, config: &RecordingConfig) {
    let Err(CreateReadError::Write(WriteError::Reply(_))) = result else {
        panic!("a body that is not JSON is a reply error for the list");
    };
    assert!(config.errors().is_empty());
}

// One send per call, and the answer differs by handler: the POST raises, the GET hands the body to its converter, and list parses it itself
#[rstest]
#[case::create(Operation::Create)]
#[case::retrieve(Operation::Retrieve)]
#[case::list(Operation::List)]
#[tokio::test]
async fn create_read_failures(#[case] operation: Operation) {
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(429, "slow").with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = call_for(operation, &params, &optional);
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    match operation {
        Operation::Create => {
            let error = check_status_failure(&result, &config);
            // the POST raises for the status before its converter reads the body, so the body text is the message
            assert_eq!(error.message, "slow");
            assert_eq!(
                error.headers.get("content-length"),
                Some(&Json::Text("4".to_owned()))
            );
        }
        Operation::Retrieve => {
            let error = check_status_failure(&result, &config);
            // the GET raises no status error, so the converter's parse error is the message, not the body text
            assert_eq!(
                error.message,
                serde_json::from_str::<Json>("slow")
                    .expect_err("slow is not a JSON value")
                    .to_string()
            );
        }
        Operation::List => check_list_failure(&result, &config),
    }
    assert_eq!(wire.requests().len(), 1);
}

// The status error's text is the body, and bytes that cannot be UTF-8 text are counted instead, for the body and a header value
#[tokio::test]
async fn create_status_failure_counts_bytes_that_are_not_utf8() {
    let events = Events::new();
    let mut reply = Reply::json(429, "").with_header("retry-after", "3");
    reply.body = vec![0xff, 0xfe];
    reply.headers.insert(
        "x-binary",
        HeaderValue::from_bytes(&[0xff, 0xfe]).expect("the bytes form a header value"),
    );
    let wire = Wire::replying(reply, &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = call_for(Operation::Create, &params, &optional);
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let error = check_status_failure(&result, &config);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
    assert_eq!(
        error.headers.get("x-binary"),
        Some(&Json::Text("2 bytes that are not UTF-8".to_owned()))
    );
}

#[rstest]
#[case::create(Operation::Create)]
#[case::retrieve(Operation::Retrieve)]
#[case::list(Operation::List)]
#[tokio::test]
async fn transport_failure_is_a_synthesized_500(#[case] operation: Operation) {
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = call_for(operation, &params, &optional);
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a lost transport is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the client is shutting down and takes no new requests"
    );
    assert!(error.headers.is_empty());
    assert_eq!(wire.requests().len(), 1);
}

fn check_timeout_message(mode: ClientKind, message: &str) {
    match mode {
        ClientKind::Sync => assert_eq!(
            message,
            "litellm.Timeout: Connection timed out after None seconds."
        ),
        ClientKind::Asynchronous => {
            assert!(message.starts_with(
                "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken="
            ));
            assert!(message.ends_with(" seconds"));
            assert!(!message.contains("time taken=0.000"));
            let elapsed = message
                .split("time taken=")
                .nth(1)
                .expect("the message carries the elapsed time")
                .trim_end_matches(" seconds");
            assert_eq!(elapsed.split('.').nth(1).map(str::len), Some(3));
        }
    }
}

// The POST handlers wrap a timeout into litellm.Timeout with 408; the sync one never passed a timeout, so its message says None
#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn create_timeout_is_a_408(#[case] mode: ClientKind) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], STORE),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let config = RecordingConfig::new(
        &format!("http://{}/v1/vector_stores", serving.address),
        &events,
    );
    let deps = CreateReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let mut call_target = target(&params);
    call_target.timeout = Some(Duration::from_millis(100));
    let call = CreateReadCall::Create(CreateCall {
        target: call_target,
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, mode, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert!(error.headers.is_empty());
    check_timeout_message(mode, &error.message);
}

// The GET handlers catch the raw timeout, which _handle_error reads as a synthesized 500 with the exception's own text
#[tokio::test]
async fn read_timeout_is_a_synthesized_500() {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], STORE),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let config = RecordingConfig::new(
        &format!("http://{}/v1/vector_stores", serving.address),
        &events,
    );
    let deps = CreateReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: target(&params),
        vector_store_id: "vs_a",
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Provider(error))) = result else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the request ran past its read timeout or total deadline"
    );
    assert!(error.headers.is_empty());
}

#[tokio::test]
async fn header_failure_makes_no_request() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let bad = caller_headers(&[("X-Trace", "bad\nvalue")]);
    let mut call_target = target(&params);
    call_target.extra_headers = Some(&bad);
    let call = CreateReadCall::Create(CreateCall {
        target: call_target,
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::HeaderValue { name, .. })) = result else {
        panic!("a header a value cannot carry fails the wire headers");
    };
    assert_eq!(name, "X-Trace");
    assert_eq!(
        events.recorded(),
        vec!["client", "capability", "headers", "url", "log"]
    );
    assert!(config.errors().is_empty());
    assert!(wire.requests().is_empty());
}

// a base that cannot be a URL fails with its own error after the provider headers and before the pre-call
#[tokio::test]
async fn unparsable_base_url_makes_no_request() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("not a url", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let call = CreateReadCall::Retrieve(RetrieveCall {
        target: target(&params),
        vector_store_id: "vs_a",
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_failure");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::RequestUrl { url, .. })) = result else {
        panic!("a base that cannot be a URL fails the URL step");
    };
    assert_eq!(url, "not a url");
    assert_eq!(events.recorded(), vec!["client", "headers", "url"]);
    assert!(config.errors().is_empty());
    assert!(wire.requests().is_empty());
}

#[tokio::test]
async fn log_failure_makes_no_request() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores", &events);
    let deps = deps(&config, &source, None);
    let params = params("http://unused.test");
    let optional = CreateParams::default();
    let call = CreateReadCall::Create(CreateCall {
        target: target(&params),
        optional_params: &optional,
        extra_body: None,
    });
    let mut fixture = Fixture::new("vector_store_failure");
    fixture.state.request_logging.raw_request_only = true;
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(CreateReadError::Write(WriteError::Logging(PrePostError::RawRequestCaptured))) = result
    else {
        panic!("a captured raw request stops the send");
    };
    assert!(wire.requests().is_empty());
}
