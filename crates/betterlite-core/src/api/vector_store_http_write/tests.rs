#![cfg(test)]

mod defaults;
mod modes;
mod pre_send;
mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use self::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire, body_bytes,
    body_json, caller_headers, extra_body, openai_config, openai_settings, params, update_params,
};
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, ResponseFacts, WriteError, handle_error, header_text,
};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, DeleteCall, UpdateCall, WriteCall, WriteDeps, WriteTarget,
    WriteTransport, send,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;
const CREATED: &str = r#"{"id":"vs_a","object":"vector_store","name":"replacement"}"#;

const fn target<'call>(
    vector_store_id: &'call str,
    params: &'call WriteParams,
    extra_headers: Option<&'call BTreeMap<String, String>>,
    timeout: Option<Duration>,
) -> WriteTarget<'call> {
    WriteTarget {
        vector_store_id,
        params,
        extra_headers,
        timeout,
    }
}

async fn run<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    events: &Events,
) -> Result<Json, WriteError>
where
    C: WriteConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let prepared = fixture.prepare(deps, mode, call, events)?;
    send(prepared).await
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn update_http_order(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, CREATED), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_update");
    let optional = update_params(
        r#"{"name":"notes","file_ids":["file_a","file_b"],"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"metadata":{"keep":"yes","hidden_params":"secret","count":2}}"#,
    );
    {
        let call = WriteCall::Update(UpdateCall {
            target: target("vs/a", &params, None, None),
            optional_params: &optional,
            extra_body: None,
        });
        let value = run(&mut fixture, &deps, mode, &call, &events)
            .await
            .expect("the update succeeds");
        assert_eq!(value, json(CREATED));
    };
    let many: IndexMap<String, Json> = (1..=18_u32)
        .map(|index| (format!("k{index:02}"), Json::Text(format!("v{index}"))))
        .collect();
    let capped = UpdateParams {
        name: None,
        file_ids: None,
        expires_after: None,
        chunking_strategy: None,
        metadata: Some(Some(many)),
    };
    {
        let call = WriteCall::Update(UpdateCall {
            target: target("vs/a", &params, None, None),
            optional_params: &capped,
            extra_body: None,
        });
        run(&mut fixture, &deps, mode, &call, &events)
            .await
            .expect("the capped update succeeds");
    };
    let extra =
        extra_body(r#"{"metadata":{"count":2},"name":"replacement","client_tag":"ticket-7"}"#);
    {
        let call = WriteCall::Update(UpdateCall {
            target: target("vs/a", &params, None, None),
            optional_params: &optional,
            extra_body: Some(&extra),
        });
        run(&mut fixture, &deps, mode, &call, &events)
            .await
            .expect("the overridden update succeeds");
    };
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs%2Fa");
    assert_eq!(
        body_json(&requests[0]),
        json(
            r#"{"name":"notes","file_ids":["file_a","file_b"],"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"metadata":{"keep":"yes"}}"#
        )
    );
    let Json::Object(capped_body) = body_json(&requests[1]) else {
        panic!("the body is an object");
    };
    let Some(Json::Object(metadata)) = capped_body.get("metadata") else {
        panic!("the capped metadata is an object");
    };
    assert_eq!(metadata.len(), 16);
    assert_eq!(metadata.keys().next().map(String::as_str), Some("k01"));
    assert_eq!(metadata.keys().last().map(String::as_str), Some("k16"));
    assert_eq!(
        body_json(&requests[2]),
        json(
            r#"{"name":"replacement","file_ids":["file_a","file_b"],"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"metadata":{"count":2},"client_tag":"ticket-7"}"#
        )
    );
}

#[rstest]
#[case::without_beta(None, "assistants=v2")]
#[case::with_beta(Some("assistants=v1"), "assistants=v1")]
#[tokio::test]
async fn write_headers(#[case] beta: Option<&str>, #[case] expected_beta: &str) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, CREATED), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let optional = UpdateParams::default();
    let mut pairs = vec![
        ("Authorization", "Bearer sk-caller"),
        ("Content-Type", "text/plain"),
        ("X-Trace", "trace-a"),
    ];
    if let Some(chosen) = beta {
        pairs.push(("OpenAI-Beta", chosen));
    }
    let headers = caller_headers(&pairs);
    let mut fixture = Fixture::new("vector_store_update");
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &params, Some(&headers), None),
        optional_params: &optional,
        extra_body: None,
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the update succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let sent = |name: &str| {
        let value = requests[0].headers.get(name)?;
        value.to_str().ok()
    };
    assert_eq!(sent("authorization"), Some("Bearer sk-provider"));
    assert_eq!(sent("content-type"), Some("application/json"));
    assert_eq!(sent("openai-beta"), Some(expected_beta));
    assert_eq!(sent("x-trace"), Some("trace-a"));
    let logged = fixture
        .logged("headers")
        .expect("the pre-call logs the validated headers");
    assert_eq!(
        logged,
        &json(&format!(
            r#"{{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"{expected_beta}","X-Trace":"trace-a"}}"#
        ))
    );
}

#[derive(Debug, Clone, Copy)]
enum DeleteCase {
    Mapping,
    Array,
    Malformed,
}

#[rstest]
#[case::mapping(DeleteCase::Mapping)]
#[case::array(DeleteCase::Array)]
#[case::malformed(DeleteCase::Malformed)]
#[tokio::test]
async fn delete_http_ignores_body(#[case] case: DeleteCase) {
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let events = Events::new();
    let (answer, expected) = match case {
        DeleteCase::Mapping => (
            Reply::json(
                200,
                r#"{"id":"vs_a","object":"vector_store.deleted","deleted":true}"#,
            ),
            Some(json(
                r#"{"id":"vs_a","object":"vector_store.deleted","deleted":true}"#,
            )),
        ),
        DeleteCase::Array => (Reply::json(200, "[1,2]"), Some(json("[1,2]"))),
        DeleteCase::Malformed => (Reply::json(200, "{not json"), None),
    };
    let wire = Wire::replying(answer, &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let ignored = extra_body(r#"{"ignored":true}"#);
    let mut fixture = Fixture::new("vector_store_delete");
    let call = WriteCall::Delete(DeleteCall {
        target: target("vs_a", &params, None, None),
        extra_body: Some(&ignored),
    });
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    if let Some(reply) = expected {
        assert_eq!(result.expect("the delete succeeds"), reply);
    } else {
        assert!(matches!(result, Err(WriteError::Reply(_))));
        assert!(config.errors().is_empty());
    }
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::DELETE);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs_a");
    assert_eq!(body_bytes(&requests[0]), None);
    assert!(fixture.logged("complete_input_dict").is_none());
}

#[derive(Debug, Clone, Copy)]
enum FailureCase {
    Http400,
    Http400EmptyBody,
    Http429,
    ClosedTransport,
    NoStatus,
    StringStatus,
    IntegerStatus,
    OutOfRangeStatus,
    EmptyHeaders,
    ResponseText,
    ResponseWithoutText,
    NoConfig,
    MalformedReply,
    TimeoutAsyncUpdate,
    TimeoutSyncUpdate,
    TimeoutAsyncDefault,
    TimeoutAsyncDelete,
    TimeoutSyncDelete,
}

#[rstest]
#[case::http_400(FailureCase::Http400)]
#[case::http_400_empty_body(FailureCase::Http400EmptyBody)]
#[case::http_429(FailureCase::Http429)]
#[case::closed_transport(FailureCase::ClosedTransport)]
#[case::no_status(FailureCase::NoStatus)]
#[case::string_status(FailureCase::StringStatus)]
#[case::integer_status(FailureCase::IntegerStatus)]
#[case::out_of_range_status(FailureCase::OutOfRangeStatus)]
#[case::empty_headers(FailureCase::EmptyHeaders)]
#[case::response_text(FailureCase::ResponseText)]
#[case::response_without_text(FailureCase::ResponseWithoutText)]
#[case::no_config(FailureCase::NoConfig)]
#[case::malformed_reply(FailureCase::MalformedReply)]
#[case::timeout_async_update(FailureCase::TimeoutAsyncUpdate)]
#[case::timeout_sync_update(FailureCase::TimeoutSyncUpdate)]
#[case::timeout_async_default(FailureCase::TimeoutAsyncDefault)]
#[case::timeout_async_delete(FailureCase::TimeoutAsyncDelete)]
#[case::timeout_sync_delete(FailureCase::TimeoutSyncDelete)]
#[tokio::test]
async fn transport_failure(#[case] case: FailureCase) {
    match case {
        FailureCase::Http429 => {
            let settings = openai_settings();
            let config = openai_config(&settings, "sk-provider");
            let events = Events::new();
            let wire = Wire::replying(
                Reply::json(429, "slow down")
                    .with_header("retry-after", "3")
                    .with_header("content-type", "text/plain")
                    .with_header("content-length", "9"),
                &events,
            );
            let source = Source::new(wire.clone());
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://mock.test/v1");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a 429 is a provider error");
            };
            assert_eq!(error.status_code, 429);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, "slow down");
            assert_eq!(
                error.headers,
                HeaderMap::from([
                    ("content-length".to_owned(), Json::Text("9".to_owned()),),
                    (
                        "content-type".to_owned(),
                        Json::Text("text/plain".to_owned()),
                    ),
                    ("retry-after".to_owned(), Json::Text("3".to_owned())),
                ])
            );
            assert_eq!(wire.requests().len(), 1);
        }
        FailureCase::ClosedTransport => {
            let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
            let events = Events::new();
            let wire = Wire::failing(TransportError::Closed, &events);
            let source = Source::new(wire.clone());
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a transport failure is a provider error");
            };
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
            assert_eq!(error.message, TransportError::Closed.to_string());
            assert!(error.headers.is_empty());
            assert_eq!(wire.requests().len(), 1);
        }
        FailureCase::NoStatus => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: None,
                    text: None,
                    headers: None,
                    response: None,
                    display: "boom".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
            assert_eq!(error.message, "boom");
            assert!(error.headers.is_empty());
            assert_eq!(
                config.errors(),
                vec![("boom".to_owned(), 500_u16, HeaderMap::new())]
            );
        }
        FailureCase::StringStatus => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: Some(Json::Text("429".to_owned())),
                    text: None,
                    headers: None,
                    response: None,
                    display: "boom".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
        }
        FailureCase::IntegerStatus => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: Some(Json::Number(Number::from(503_u16))),
                    text: Some("service unavailable".to_owned()),
                    headers: None,
                    response: None,
                    display: "boom".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.status_code, 503);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, "service unavailable");
        }
        FailureCase::OutOfRangeStatus => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: Some(Json::Number(Number::from(70_000_u32))),
                    text: Some("too high".to_owned()),
                    headers: None,
                    response: None,
                    display: "boom".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
            assert_eq!(error.message, "too high");
            assert_eq!(
                config.errors(),
                vec![("too high".to_owned(), 500_u16, HeaderMap::new())]
            );
        }
        FailureCase::EmptyHeaders => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: Some(Json::Number(Number::from(500_u16))),
                    text: Some("local".to_owned()),
                    headers: Some(HeaderMap::new()),
                    response: Some(ResponseFacts {
                        status_code: 500,
                        text: Some("remote".to_owned()),
                        headers: Some(HeaderMap::from([(
                            "retry-after".to_owned(),
                            Json::Text("3".to_owned()),
                        )])),
                    }),
                    display: "local".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.message, "remote");
            assert!(error.headers.is_empty());
            assert_eq!(
                config.errors(),
                vec![("remote".to_owned(), 500_u16, HeaderMap::new())]
            );
        }
        FailureCase::ResponseText => {
            let config = RecordingConfig::new("http://unused.test");
            let expected =
                HeaderMap::from([("retry-after".to_owned(), Json::Text("3".to_owned()))]);
            let error = handle_error(
                &FailureFacts {
                    response_status: None,
                    status: Some(Json::Number(Number::from(500_u16))),
                    text: Some("local".to_owned()),
                    headers: None,
                    response: Some(ResponseFacts {
                        status_code: 500,
                        text: Some("remote".to_owned()),
                        headers: Some(expected.clone()),
                    }),
                    display: "local".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.message, "remote");
            assert_eq!(error.headers, expected);
        }
        FailureCase::ResponseWithoutText => {
            let config = RecordingConfig::new("http://unused.test");
            let error = handle_error(
                &FailureFacts {
                    response_status: Some(500),
                    status: None,
                    text: Some("local".to_owned()),
                    headers: None,
                    response: Some(ResponseFacts {
                        status_code: 500,
                        text: None,
                        headers: None,
                    }),
                    display: "local".to_owned(),
                },
                Some(&config),
            );
            assert_eq!(error.status_code, 500);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, "");
            assert!(error.headers.is_empty());
        }
        FailureCase::NoConfig => {
            let error = handle_error(
                &FailureFacts {
                    response_status: Some(503),
                    status: None,
                    text: None,
                    headers: None,
                    response: Some(ResponseFacts {
                        status_code: 503,
                        text: Some("x".to_owned()),
                        headers: Some(HeaderMap::from([(
                            "retry-after".to_owned(),
                            Json::Text("3".to_owned()),
                        )])),
                    }),
                    display: String::new(),
                },
                None,
            );
            assert_eq!(error.status_code, 503);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, "x");
            assert_eq!(
                error.headers,
                HeaderMap::from([("retry-after".to_owned(), Json::Text("3".to_owned()))])
            );
        }
        FailureCase::TimeoutAsyncUpdate => {
            let serving = serve(
                reply("200 OK", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(500),
            )
            .expect("the fixture serves");
            let source = ProxiedSource::new(IMPATIENT);
            let config =
                RecordingConfig::new(&format!("http://{}/v1/vector_stores", serving.address));
            let events = Events::new();
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, Some(Duration::from_millis(100))),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a timeout is a provider error");
            };
            assert_eq!(error.status_code, 408);
            assert!(!error.status_code_is_synthesized);
            assert!(error.message.starts_with(
                "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken="
            ));
            assert!(error.message.ends_with(" seconds"));
            assert!(!error.message.contains("time taken=0.000"));
            let elapsed = error
                .message
                .split("time taken=")
                .nth(1)
                .expect("the message carries the elapsed time")
                .trim_end_matches(" seconds");
            assert_eq!(elapsed.split('.').nth(1).map(str::len), Some(3));
            assert!(error.headers.is_empty());
        }
        FailureCase::TimeoutSyncUpdate => {
            let serving = serve(
                reply("200 OK", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(500),
            )
            .expect("the fixture serves");
            let source = ProxiedSource::new(IMPATIENT);
            let config =
                RecordingConfig::new(&format!("http://{}/v1/vector_stores", serving.address));
            let events = Events::new();
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, Some(Duration::from_millis(100))),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a timeout is a provider error");
            };
            assert_eq!(error.status_code, 408);
            assert_eq!(
                error.message,
                "litellm.Timeout: Connection timed out after None seconds."
            );
        }
        FailureCase::TimeoutAsyncDelete => {
            let serving = serve(
                reply("200 OK", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(500),
            )
            .expect("the fixture serves");
            let source = ProxiedSource::new(IMPATIENT);
            let config =
                RecordingConfig::new(&format!("http://{}/v1/vector_stores", serving.address));
            let events = Events::new();
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let mut fixture = Fixture::new("vector_store_delete");
            let call = WriteCall::Delete(DeleteCall {
                target: target("vs_a", &params, None, Some(Duration::from_millis(100))),
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("an async delete timeout is a provider error");
            };
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
            assert!(error.message.contains("read timeout"));
            assert!(error.headers.is_empty());
        }
        FailureCase::Http400 => {
            let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
            let events = Events::new();
            let wire = Wire::replying(Reply::json(400, r#"{"id":"vs_a"}"#), &events);
            let source = Source::new(wire.clone());
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://mock.test/v1");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a 400 is a provider error");
            };
            assert_eq!(error.status_code, 400);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, r#"{"id":"vs_a"}"#);
            assert_eq!(wire.requests().len(), 1);
        }
        FailureCase::Http400EmptyBody => {
            let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
            let events = Events::new();
            let wire = Wire::replying(Reply::json(400, ""), &events);
            let source = Source::new(wire.clone());
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://mock.test/v1");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a 400 is a provider error");
            };
            assert_eq!(error.status_code, 400);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(error.message, "");
            assert!(error.headers.is_empty());
            assert_eq!(wire.requests().len(), 1);
        }
        FailureCase::MalformedReply => {
            let settings = openai_settings();
            let config = openai_config(&settings, "sk-provider");
            let events = Events::new();
            let wire = Wire::replying(
                Reply::json(200, "{not json").with_header("retry-after", "3"),
                &events,
            );
            let source = Source::new(wire.clone());
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://mock.test/v1");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a reply that does not convert is a provider error");
            };
            assert_eq!(error.status_code, 200);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(
                error.headers.get("retry-after"),
                Some(&Json::Text("3".to_owned()))
            );
            assert_eq!(wire.requests().len(), 1);
        }
        FailureCase::TimeoutAsyncDefault => {
            let serving = serve(
                reply("200 OK", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(500),
            )
            .expect("the fixture serves");
            let source = ProxiedSource::new(IMPATIENT);
            let config =
                RecordingConfig::new(&format!("http://{}/v1/vector_stores", serving.address));
            let events = Events::new();
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let optional = UpdateParams::default();
            let mut fixture = Fixture::new("vector_store_update");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let result = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("the client's own read timeout is a provider error");
            };
            assert_eq!(error.status_code, 408);
            assert!(error.message.starts_with(
                "litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken="
            ));
        }
        FailureCase::TimeoutSyncDelete => {
            let serving = serve(
                reply("200 OK", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(500),
            )
            .expect("the fixture serves");
            let source = ProxiedSource::new(IMPATIENT);
            let config =
                RecordingConfig::new(&format!("http://{}/v1/vector_stores", serving.address));
            let events = Events::new();
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let params = params("http://unused.test");
            let mut fixture = Fixture::new("vector_store_delete");
            let call = WriteCall::Delete(DeleteCall {
                target: target("vs_a", &params, None, None),
                extra_body: None,
            });
            let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
            let Err(WriteError::Provider(error)) = result else {
                panic!("a sync delete timeout is a provider error");
            };
            assert_eq!(error.status_code, 408);
            assert!(!error.status_code_is_synthesized);
            assert_eq!(
                error.message,
                "litellm.Timeout: Connection timed out after None seconds."
            );
        }
    }
}

#[tokio::test]
async fn binary_bodies_and_headers_are_named_by_their_length() {
    let events = Events::new();
    let wire = Wire::replying(
        Reply {
            status: 400,
            headers: http::HeaderMap::new(),
            body: vec![0xff, 0xfe],
        },
        &events,
    );
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let optional = UpdateParams::default();
    let mut fixture = Fixture::new("vector_store_update");
    let call = WriteCall::Update(UpdateCall {
        target: target("vs_a", &params, None, None),
        optional_params: &optional,
        extra_body: None,
    });
    let result = run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await;
    let Err(WriteError::Provider(error)) = result else {
        panic!("a 400 is a provider error");
    };
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
    let value = http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("the bytes form a value");
    assert_eq!(header_text(&value), "2 bytes that are not UTF-8");
}

#[rstest]
#[case::supplied_sync(modes::ModeCase::SuppliedSync)]
#[case::supplied_asynchronous(modes::ModeCase::SuppliedAsynchronous)]
#[case::mismatch(modes::ModeCase::Mismatch)]
#[case::query_base(modes::ModeCase::QueryBase)]
#[case::logging_order(modes::ModeCase::LoggingOrder)]
#[tokio::test]
async fn execution_mode(#[case] case: modes::ModeCase) {
    modes::execution_mode_case(case).await;
}

#[rstest]
#[case::empty_params(defaults::DefaultsCase::EmptyParams)]
#[case::metadata_null(defaults::DefaultsCase::MetadataNull)]
#[case::metadata_absent(defaults::DefaultsCase::MetadataAbsent)]
#[case::metadata_empty(defaults::DefaultsCase::MetadataEmpty)]
#[case::extra_empty(defaults::DefaultsCase::ExtraEmpty)]
#[case::supplied_bypasses_pool(defaults::DefaultsCase::SuppliedBypassesPool)]
#[case::delete_omitted(defaults::DefaultsCase::DeleteOmitted)]
#[tokio::test]
async fn write_argument_defaults(#[case] case: defaults::DefaultsCase) {
    defaults::write_argument_defaults_case(case).await;
}
