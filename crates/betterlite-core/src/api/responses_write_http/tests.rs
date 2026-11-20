#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::error::ResponseResourceError;
use http::Method;
use rstest::rstest;
use secrecy::SecretString;

use self::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_json, caller_headers,
    captured_messages, capturing, delete_call, extra_body, openai_config, openai_settings, params,
    write_deps,
};
use crate::api::responses_write_http::config::{ResponsesConfig, WriteParams};
use crate::api::responses_write_http::error::WriteError;
use crate::api::responses_write_http::service::{
    CancelCall, ClientKind, ClientSource, DeleteCall, SharedSession, SuppliedClient, WriteCall,
    WriteDeps, WriteOutcome, WriteRequest, WriteTarget, send,
};
use crate::api::vector_store_http_write::error::HeaderMap;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn header(request: &WriteRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    events: &Events,
) -> Result<WriteOutcome, WriteError>
where
    C: ResponsesConfig,
    S: ClientSource,
{
    let prepared = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}

const DELETED: &str = r#"{"id":"resp_a","object":"response","deleted":true}"#;
const CANCELLED: &str = r#"{"id":"resp_a","created_at":1735689600,"object":"response","status":"cancelled","model":"gpt-5","output":[]}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Delete,
    Cancel,
}

const fn operation_call<'call>(
    operation: Operation,
    response_id: &'call str,
    params: &'call WriteParams,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    timeout: Option<Duration>,
) -> WriteCall<'call> {
    let target = WriteTarget {
        response_id,
        params,
        extra_headers,
        timeout,
    };
    match operation {
        Operation::Delete => WriteCall::Delete(DeleteCall {
            target,
            extra_body: None,
        }),
        Operation::Cancel => WriteCall::Cancel(CancelCall {
            target,
            extra_body: None,
        }),
    }
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn delete_omits_empty_body(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, DELETED), &events);
    let source = Source::new(wire.clone());
    let deps = write_deps(&config, &source, None, None);
    let write_params = params("http://mock.test/v1");
    let ignored = extra_body(r#"{"ignored":true}"#);
    let mut fixture = Fixture::new("responses");
    let call = delete_call(
        "resp/a",
        &write_params,
        None,
        Some(&ignored),
        Some(Duration::from_secs(7)),
    );
    let outcome = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the delete succeeds");
    let WriteOutcome::Deleted(deleted) = outcome else {
        panic!("a delete returns the deleted record");
    };
    assert_eq!(deleted.id.as_deref(), Some("resp_a"));
    assert_eq!(deleted.object.as_deref(), Some("response"));
    assert_eq!(deleted.deleted, Some(true));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, Method::DELETE);
        assert_eq!(
            request.url.as_str(),
            "http://mock.test/v1/responses/resp%2Fa"
        );
        assert!(matches!(&request.body, RequestBody::Empty));
        assert_eq!(request.timeout, Some(Duration::from_secs(7)));
        assert_eq!(
            header(request, "content-type").as_deref(),
            Some("application/json")
        );
        assert_eq!(
            header(request, "authorization").as_deref(),
            Some("Bearer sk-provider")
        );
    }
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/responses""#))
    );
    assert_eq!(fixture.logged("complete_input_dict"), Some(&json("{}")));
    assert_eq!(fixture.logged("extra_body"), None);
    assert_eq!(fixture.details("api_key"), None);
    assert!(fixture.logged("headers").is_some());
    let plain = caller_headers(&[("Content-Type", "text/plain")]);
    let mut typed_fixture = Fixture::new("responses");
    let typed_call = delete_call("resp_a", &write_params, Some(&plain), None, None);
    run(&mut typed_fixture, &deps, mode, &typed_call, &events)
        .await
        .expect("the delete with a caller media type succeeds");
    assert_eq!(wire.requests().len(), 2);
    assert_eq!(
        header(&wire.requests()[1], "content-type").as_deref(),
        Some("text/plain")
    );
    let recording = RecordingConfig::new("http://mock.test/v1/responses");
    let interface_wire = Wire::replying(Reply::json(200, DELETED), &events);
    let interface_source = Source::new(interface_wire.clone());
    let interface_deps = WriteDeps {
        config: &recording,
        source: &interface_source,
        supplied: None,
        shared_session: None,
    };
    let mut interface_fixture = Fixture::new("responses");
    let empty_call = delete_call("resp_a", &write_params, None, None, None);
    run(
        &mut interface_fixture,
        &interface_deps,
        mode,
        &empty_call,
        &events,
    )
    .await
    .expect("the empty payload row succeeds");
    assert!(matches!(
        &interface_wire.requests()[0].body,
        RequestBody::Empty
    ));
    recording.set_payload(r#"{"purge":true}"#);
    let purging_call = delete_call("resp_a", &write_params, None, None, None);
    run(
        &mut interface_fixture,
        &interface_deps,
        mode,
        &purging_call,
        &events,
    )
    .await
    .expect("the nonempty payload row succeeds");
    assert_eq!(
        body_json(&interface_wire.requests()[1]),
        json(r#"{"purge":true}"#)
    );
    let bare = RecordingConfig::without_media_type("http://mock.test/v1/responses");
    let bare_wire = Wire::replying(Reply::json(200, DELETED), &events);
    let bare_source = Source::new(bare_wire.clone());
    let bare_deps = WriteDeps {
        config: &bare,
        source: &bare_source,
        supplied: None,
        shared_session: None,
    };
    let mut bare_fixture = Fixture::new("responses");
    let bare_call = delete_call("resp_a", &write_params, None, None, None);
    run(&mut bare_fixture, &bare_deps, mode, &bare_call, &events)
        .await
        .expect("the delete without a provider media type succeeds");
    assert_eq!(
        header(&bare_wire.requests()[0], "content-type").as_deref(),
        Some("application/json")
    );
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn cancel_posts_empty_object(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-env");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(200, CANCELLED).with_header("content-type", "application/json"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = write_deps(&config, &source, None, None);
    let write_params = WriteParams {
        api_base: Some(String::from("http://mock.test/v1")),
        api_key: Some(SecretString::from("sk-provider")),
        ssl_verify: None,
    };
    let ignored = extra_body(r#"{"reason":"ignored"}"#);
    let caller = caller_headers(&[
        ("Authorization", "Bearer sk-caller"),
        ("X-Trace", "trace-a"),
    ]);
    let mut fixture = Fixture::new("responses");
    let call = WriteCall::Cancel(CancelCall {
        target: WriteTarget {
            response_id: "resp_a",
            params: &write_params,
            extra_headers: Some(&caller),
            timeout: None,
        },
        extra_body: Some(&ignored),
    });
    let outcome = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the cancel succeeds");
    let WriteOutcome::Cancelled(typed) = outcome else {
        panic!("a cancel returns the cancelled response");
    };
    assert_eq!(typed.response.id, "resp_a");
    assert_eq!(typed.response.status.as_deref(), Some("cancelled"));
    assert_eq!(typed.response.model.as_deref(), Some("gpt-5"));
    assert_eq!(
        typed.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        typed.additional_headers.get("llm_provider-content-type"),
        Some(&Json::Text(String::from("application/json")))
    );
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, Method::POST);
        assert_eq!(
            request.url.as_str(),
            "http://mock.test/v1/responses/resp_a/cancel"
        );
        assert_eq!(body_json(request), json("{}"));
        assert_eq!(
            header(request, "authorization").as_deref(),
            Some("Bearer sk-provider")
        );
        assert_eq!(header(request, "x-trace").as_deref(), Some("trace-a"));
    }
    assert_eq!(fixture.input(), Some(&json(r#""resp_a""#)));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/responses/resp_a/cancel""#))
    );
    assert_eq!(fixture.logged("complete_input_dict"), Some(&json("{}")));
    assert_eq!(fixture.logged("extra_body"), None);
    assert_eq!(fixture.logged("api_key"), None);
    assert_eq!(fixture.details("api_key"), None);
    let Json::Object(headers) = fixture
        .logged("headers")
        .expect("the pre-call carries the headers")
    else {
        panic!("the pre-call headers are an object");
    };
    assert_eq!(
        headers.get("X-Trace"),
        Some(&Json::Text(String::from("trace-a")))
    );
    let bare = RecordingConfig::without_media_type("http://mock.test/v1/responses");
    let bare_wire = Wire::replying(
        Reply::json(200, CANCELLED).with_header("content-type", "application/json"),
        &events,
    );
    let bare_source = Source::new(bare_wire.clone());
    let bare_deps = WriteDeps {
        config: &bare,
        source: &bare_source,
        supplied: None,
        shared_session: None,
    };
    let mut bare_fixture = Fixture::new("responses");
    let bare_call = operation_call(Operation::Cancel, "resp_a", &write_params, None, None);
    run(&mut bare_fixture, &bare_deps, mode, &bare_call, &events)
        .await
        .expect("the cancel without a provider media type succeeds");
    // LiteLLM sets the media type only for delete, never for cancel
    assert_eq!(header(&bare_wire.requests()[0], "content-type"), None);
}

#[rstest]
#[case::delete(Operation::Delete)]
#[case::cancel(Operation::Cancel)]
#[tokio::test]
async fn write_transport_failures(#[case] operation: Operation) {
    let config = RecordingConfig::new("http://mock.test/v1/responses");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(429, "slow down")
            .with_header("retry-after", "3")
            .with_header("content-type", "text/plain")
            .with_header("content-encoding", "gzip"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = write_deps(&config, &source, None, None);
    let write_params = params("http://unused.test");
    let mut fixture = Fixture::new("responses");
    let call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Provider(provider_error)) =
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await
    else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(provider_error.status_code, 429);
    assert!(!provider_error.status_code_is_synthesized);
    assert_eq!(provider_error.message, "slow down");
    assert_eq!(
        provider_error.headers.get("retry-after"),
        Some(&Json::Text(String::from("3")))
    );
    assert_eq!(
        provider_error.headers.get("content-type"),
        Some(&Json::Text(String::from("text/plain")))
    );
    assert_eq!(
        provider_error.headers.get("content-length"),
        Some(&Json::Text(String::from("9")))
    );
    // The masked error drops the encoding of the original reply
    assert_eq!(provider_error.headers.get("content-encoding"), None);
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        config.errors(),
        vec![(
            String::from("slow down"),
            429,
            provider_error.headers.clone()
        )]
    );
    let empty = RecordingConfig::new("http://mock.test/v1/responses");
    let empty_wire = Wire::replying(
        Reply::json(503, "").with_header("content-length", "0"),
        &events,
    );
    let empty_source = Source::new(empty_wire.clone());
    let empty_deps = WriteDeps {
        config: &empty,
        source: &empty_source,
        supplied: None,
        shared_session: None,
    };
    let mut empty_fixture = Fixture::new("responses");
    let empty_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Provider(empty_error)) = run(
        &mut empty_fixture,
        &empty_deps,
        ClientKind::Sync,
        &empty_call,
        &events,
    )
    .await
    else {
        panic!("a 503 with no body is a provider error");
    };
    assert_eq!(empty_error.status_code, 503);
    assert!(empty_error.message.is_empty());
    // The masked rebuild carries a content-length only when the decoded body is non-empty
    assert_eq!(empty_error.headers.get("content-length"), None);
    assert_eq!(empty_wire.requests().len(), 1);
    let unrouted = RecordingConfig::new("not a url");
    let unrouted_wire = Wire::replying(Reply::json(200, DELETED), &events);
    let unrouted_source = Source::new(unrouted_wire.clone());
    let unrouted_deps = WriteDeps {
        config: &unrouted,
        source: &unrouted_source,
        supplied: None,
        shared_session: None,
    };
    let mut unrouted_fixture = Fixture::new("responses");
    let unrouted_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::RequestUrl { .. }) = run(
        &mut unrouted_fixture,
        &unrouted_deps,
        ClientKind::Sync,
        &unrouted_call,
        &events,
    )
    .await
    else {
        panic!("a base that is not a URL fails before the send");
    };
    assert!(unrouted_wire.requests().is_empty());
    let refused_headers = caller_headers(&[("bad name", "value")]);
    let refused = RecordingConfig::new("http://mock.test/v1/responses");
    let refused_wire = Wire::replying(Reply::json(200, DELETED), &events);
    let refused_source = Source::new(refused_wire.clone());
    let refused_deps = WriteDeps {
        config: &refused,
        source: &refused_source,
        supplied: None,
        shared_session: None,
    };
    let mut refused_fixture = Fixture::new("responses");
    let refused_call = operation_call(
        operation,
        "resp_a",
        &write_params,
        Some(&refused_headers),
        None,
    );
    let Err(WriteError::HeaderName { .. }) = run(
        &mut refused_fixture,
        &refused_deps,
        ClientKind::Sync,
        &refused_call,
        &events,
    )
    .await
    else {
        panic!("a header name HTTP cannot carry fails before the send");
    };
    assert!(refused_wire.requests().is_empty());
    let malformed_headers = caller_headers(&[("x-bad", "bad\nvalue")]);
    let malformed = RecordingConfig::new("http://mock.test/v1/responses");
    let malformed_wire = Wire::replying(Reply::json(200, DELETED), &events);
    let malformed_source = Source::new(malformed_wire.clone());
    let malformed_deps = WriteDeps {
        config: &malformed,
        source: &malformed_source,
        supplied: None,
        shared_session: None,
    };
    let mut malformed_fixture = Fixture::new("responses");
    let malformed_call = operation_call(
        operation,
        "resp_a",
        &write_params,
        Some(&malformed_headers),
        None,
    );
    let Err(WriteError::HeaderValue { .. }) = run(
        &mut malformed_fixture,
        &malformed_deps,
        ClientKind::Sync,
        &malformed_call,
        &events,
    )
    .await
    else {
        panic!("a header value HTTP cannot carry fails before the send");
    };
    assert!(malformed_wire.requests().is_empty());
    let mut garbled = Reply::json(429, "slow down");
    garbled.body = vec![0xff_u8, 0xfe];
    let garbled_wire = Wire::replying(garbled, &events);
    let garbled_config = RecordingConfig::new("http://mock.test/v1/responses");
    let garbled_source = Source::new(garbled_wire.clone());
    let garbled_deps = WriteDeps {
        config: &garbled_config,
        source: &garbled_source,
        supplied: None,
        shared_session: None,
    };
    let mut garbled_fixture = Fixture::new("responses");
    let garbled_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Provider(garbled_error)) = run(
        &mut garbled_fixture,
        &garbled_deps,
        ClientKind::Sync,
        &garbled_call,
        &events,
    )
    .await
    else {
        panic!("a non-UTF-8 error body is still a provider error");
    };
    assert_eq!(garbled_error.status_code, 429);
    assert_eq!(garbled_error.message, "2 bytes that are not UTF-8");
    assert_eq!(
        garbled_error.headers.get("content-length"),
        Some(&Json::Text(String::from("2")))
    );
    let rejected = RecordingConfig::new("http://mock.test/v1/responses");
    let rejected_wire = Wire::replying(Reply::json(400, DELETED), &events);
    let rejected_source = Source::new(rejected_wire.clone());
    let rejected_deps = WriteDeps {
        config: &rejected,
        source: &rejected_source,
        supplied: None,
        shared_session: None,
    };
    let mut rejected_fixture = Fixture::new("responses");
    let rejected_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Provider(rejected_error)) = run(
        &mut rejected_fixture,
        &rejected_deps,
        ClientKind::Sync,
        &rejected_call,
        &events,
    )
    .await
    else {
        panic!("a 400 is a provider error even when its body is a valid record");
    };
    assert_eq!(rejected_error.status_code, 400);
    assert!(!rejected_error.status_code_is_synthesized);
    assert_eq!(rejected_error.message, DELETED);
    assert_eq!(rejected_wire.requests().len(), 1);
    let invalid = RecordingConfig::new("http://mock.test/v1/responses");
    let invalid_wire = Wire::replying(Reply::json(200, "not json"), &events);
    let invalid_source = Source::new(invalid_wire.clone());
    let invalid_deps = WriteDeps {
        config: &invalid,
        source: &invalid_source,
        supplied: None,
        shared_session: None,
    };
    let mut invalid_fixture = Fixture::new("responses");
    let invalid_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Resource(resource_error)) = run(
        &mut invalid_fixture,
        &invalid_deps,
        ClientKind::Sync,
        &invalid_call,
        &events,
    )
    .await
    else {
        panic!("a 200 that is not JSON fails the transform");
    };
    assert_eq!(resource_error.http_status(), Some(200));
    assert!(invalid.errors().is_empty());
    assert_eq!(invalid_wire.requests().len(), 1);
    if matches!(operation, Operation::Cancel) {
        let missing_wire = Wire::replying(Reply::json(200, r#"{"object":"response"}"#), &events);
        let missing_source = Source::new(missing_wire.clone());
        let missing_deps = WriteDeps {
            config: &invalid,
            source: &missing_source,
            supplied: None,
            shared_session: None,
        };
        let mut missing_fixture = Fixture::new("responses");
        let missing_call = operation_call(operation, "resp_a", &write_params, None, None);
        let Err(WriteError::Resource(ResponseResourceError::Record { .. })) = run(
            &mut missing_fixture,
            &missing_deps,
            ClientKind::Sync,
            &missing_call,
            &events,
        )
        .await
        else {
            panic!("a 200 without the required response fields fails the record");
        };
        assert!(invalid.errors().is_empty());
        assert_eq!(missing_wire.requests().len(), 1);
    }
    let failing = RecordingConfig::new("http://mock.test/v1/responses");
    let failed_wire = Wire::failing(TransportError::Closed, &events);
    let failed_source = Source::new(failed_wire.clone());
    let failed_deps = WriteDeps {
        config: &failing,
        source: &failed_source,
        supplied: None,
        shared_session: None,
    };
    let mut failed_fixture = Fixture::new("responses");
    let failed_call = operation_call(operation, "resp_a", &write_params, None, None);
    let Err(WriteError::Provider(failure)) = run(
        &mut failed_fixture,
        &failed_deps,
        ClientKind::Sync,
        &failed_call,
        &events,
    )
    .await
    else {
        panic!("a closed client is a provider error");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert_eq!(failure.message, TransportError::Closed.to_string());
    assert!(failure.headers.is_empty());
    assert_eq!(
        failing.errors(),
        vec![(TransportError::Closed.to_string(), 500, HeaderMap::new())]
    );
}

#[tokio::test]
async fn write_client_and_logging_contract() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, CANCELLED), &events);
    let created = Wire::replying(Reply::json(200, CANCELLED), &events);
    let source = Source::new(created.clone());
    let unverified = params("http://mock.test/v1");
    let verified = WriteParams {
        api_base: Some(String::from("http://mock.test/v1")),
        api_key: None,
        ssl_verify: Some(false),
    };
    let mut fixture = Fixture::new("responses");
    {
        let deps = write_deps(
            &config,
            &source,
            Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &supplied,
            }),
            Some(SharedSession),
        );
        let call = operation_call(Operation::Delete, "resp_a", &unverified, None, None);
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("a matching injected client is reused");
    }
    assert!(source.calls().is_empty());
    assert_eq!(supplied.requests().len(), 1);
    assert!(created.requests().is_empty());
    {
        let deps = write_deps(&config, &source, None, Some(SharedSession));
        let call = operation_call(Operation::Delete, "resp_a", &verified, None, None);
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("an absent client comes from the source");
    }
    assert_eq!(source.calls(), vec![(Some(false), false)]);
    assert_eq!(created.requests().len(), 1);
    {
        let deps = write_deps(
            &config,
            &source,
            Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &supplied,
            }),
            Some(SharedSession),
        );
        let call = operation_call(Operation::Delete, "resp_a", &unverified, None, None);
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("a wrong-kind client is replaced");
    }
    assert_eq!(source.calls(), vec![(Some(false), false), (None, true)]);
    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(created.requests().len(), 2);
    let (captured, _guard) = capturing();
    {
        let deps = write_deps(&config, &source, None, Some(SharedSession));
        let call = operation_call(Operation::Delete, "resp_a", &unverified, None, None);
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("the async delete creates its client");
    }
    {
        let deps = write_deps(&config, &source, None, None);
        let call = operation_call(Operation::Cancel, "resp_a", &unverified, None, None);
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("the async cancel creates its client");
    }
    {
        let deps = write_deps(&config, &source, None, Some(SharedSession));
        let call = operation_call(Operation::Delete, "resp_a", &unverified, None, None);
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the sync delete creates its client");
    }
    let client_lines: Vec<String> = captured_messages(&captured)
        .into_iter()
        .filter(|message| message.contains("Creating HTTP client for"))
        .collect();
    assert_eq!(
        client_lines,
        vec![
            String::from("Creating HTTP client for delete_response with shared_session: Some"),
            String::from("Creating HTTP client for cancel_response with shared_session: None"),
        ]
    );
    let mut logging_fixture = Fixture::new("responses");
    let write_params = params("http://mock.test/v1");
    {
        let deps = write_deps(&config, &source, None, None);
        let call = operation_call(Operation::Delete, "resp_a", &write_params, None, None);
        run(
            &mut logging_fixture,
            &deps,
            ClientKind::Sync,
            &call,
            &events,
        )
        .await
        .expect("the delete logs its call");
    }
    assert_eq!(logging_fixture.input(), Some(&json(r#""""#)));
    assert_eq!(
        logging_fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/responses""#))
    );
    {
        let deps = write_deps(&config, &source, None, None);
        let call = WriteCall::Cancel(CancelCall {
            target: WriteTarget {
                response_id: "resp_a",
                params: &write_params,
                extra_headers: None,
                timeout: None,
            },
            extra_body: None,
        });
        run(
            &mut logging_fixture,
            &deps,
            ClientKind::Sync,
            &call,
            &events,
        )
        .await
        .expect("the cancel logs its call");
    }
    assert_eq!(logging_fixture.input(), Some(&json(r#""resp_a""#)));
    assert_eq!(
        logging_fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/responses/resp_a/cancel""#))
    );
}
