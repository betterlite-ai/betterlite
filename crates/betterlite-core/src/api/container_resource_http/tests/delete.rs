#![cfg(test)]

use rstest::rstest;

use betterlite_http::proxy::pool::PoolError;
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::api::container_contracts::records::DeleteContainerResult;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;

use super::tests::{
    API_BASE, BuiltClient, CONTAINER_ID, DELETED_JSON, ErrorSeen, FakeSource, Fixture, Logged,
    call_error, closed_error, deleted_container, done, header_map, only, params, provider_error,
    query, refused, rejected_timeout, reply, runtime, timeout_error,
};
use super::{ContainerResourceCall, ContainerResourceHttpError, Handled, TIMEOUT_MESSAGE};

fn run_delete(
    fixture: &Fixture,
    call: &ContainerResourceCall<'_, FakeSource>,
) -> DeleteContainerResult {
    done(fixture.handler().container_delete_handler(call, false)).expect("the delete succeeds")
}

fn delete_error(
    fixture: &Fixture,
    call: &ContainerResourceCall<'_, FakeSource>,
) -> ContainerResourceHttpError {
    refused(fixture.handler().container_delete_handler(call, false))
}

#[test]
fn delete_sends_a_delete_and_decodes_the_result() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, DELETED_JSON));
    let call = fixture.call();
    assert_eq!(run_delete(&fixture, &call), deleted_container());
    let request = only(&fixture.source.seen());
    assert_eq!(request.method, ContainerMethod::Delete);
    assert_eq!(request.url, format!("{API_BASE}/containers/{CONTAINER_ID}"));
    assert_eq!(
        request.headers,
        header_map(&[
            ("authorization", "Bearer sk-test"),
            ("x-provider", "validated"),
        ])
    );
    assert!(request.query.is_empty());
}

#[test]
fn delete_logs_the_transformed_call_before_sending() {
    let mut fixture = Fixture::new();
    fixture.config.transformed_url =
        Some("https://api.example/v1/containers/cntr_1?api-version=old".to_owned());
    fixture.config.params = params(r#"{"limit":"5"}"#);
    fixture.source.answer(reply(200, DELETED_JSON));
    let call = fixture.call();
    assert_eq!(run_delete(&fixture, &call), deleted_container());
    assert_eq!(
        fixture.hook.seen(),
        vec![Logged {
            url: "https://api.example/v1/containers/cntr_1?api-version=old".to_owned(),
            headers: header_map(&[
                ("authorization", "Bearer sk-test"),
                ("x-provider", "validated"),
            ]),
            params: header_map(&[("limit", "5")]),
            container_id: CONTAINER_ID.to_owned(),
        }]
    );
    assert_eq!(
        only(&fixture.source.seen()).url,
        "https://api.example/v1/containers/cntr_1?limit=5"
    );
}

#[test]
fn delete_replaces_the_urls_own_query_with_the_overlaid_params() {
    let mut fixture = Fixture::new();
    fixture.config.transformed_url =
        Some("https://api.example/v1/containers/cntr_1?api-version=old".to_owned());
    fixture.config.params = params(r#"{"limit":"5"}"#);
    fixture.source.answer(reply(200, DELETED_JSON));
    let extra = query(&[("force", ParamValue::Text("true".to_owned()))]);
    let call = ContainerResourceCall {
        extra_query: Some(&extra),
        ..fixture.call()
    };
    run_delete(&fixture, &call);
    let request = only(&fixture.source.seen());
    assert_eq!(
        request.url,
        "https://api.example/v1/containers/cntr_1?limit=5&force=true"
    );
    assert!(request.query.is_empty());
}

#[test]
fn delete_keeps_the_urls_own_query_when_the_params_are_empty() {
    let mut fixture = Fixture::new();
    fixture.config.transformed_url =
        Some("https://api.example/v1/containers/cntr_1?api-version=old".to_owned());
    fixture.source.answer(reply(200, DELETED_JSON));
    let call = fixture.call();
    run_delete(&fixture, &call);
    let request = only(&fixture.source.seen());
    assert_eq!(
        request.url,
        "https://api.example/v1/containers/cntr_1?api-version=old"
    );
    assert!(request.query.is_empty());
}

#[test]
fn delete_reports_a_url_that_cannot_carry_the_params() {
    let mut fixture = Fixture::new();
    fixture.config.transformed_url = Some("not a url".to_owned());
    fixture.config.params = params(r#"{"limit":"5"}"#);
    let call = fixture.call();
    let error = call_error(delete_error(&fixture, &call));
    assert!(
        matches!(error, ContainerFileHttpError::BaseUrl { .. }),
        "{error}"
    );
    assert!(fixture.source.seen().is_empty());
}

#[test]
fn delete_lays_extra_headers_over_the_validation_answer() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, DELETED_JSON));
    let extra = header_map(&[("authorization", "Bearer extra"), ("x-extra", "yes")]);
    let call = ContainerResourceCall {
        extra_headers: Some(&extra),
        ..fixture.call()
    };
    run_delete(&fixture, &call);
    let request = only(&fixture.source.seen());
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer extra")
    );
    assert_eq!(
        request.headers.get("x-provider").map(String::as_str),
        Some("validated")
    );
    assert_eq!(fixture.config.keys(), vec![Some("sk-test".to_owned())]);
}

#[test]
fn delete_uses_the_supplied_blocking_client() {
    let fixture = Fixture::new();
    fixture.supplied.answer(reply(200, DELETED_JSON));
    let call = ContainerResourceCall {
        blocking_client: Some(&fixture.supplied),
        ..fixture.call()
    };
    assert_eq!(run_delete(&fixture, &call), deleted_container());
    assert_eq!(fixture.supplied.seen().len(), 1);
    assert!(fixture.source.built().is_empty());
    assert!(fixture.source.seen().is_empty());
}

#[test]
fn delete_reports_a_client_the_source_refuses() {
    let fixture = Fixture::new();
    fixture.source.refuse();
    let call = fixture.call();
    let error = call_error(delete_error(&fixture, &call));
    assert!(
        matches!(error, ContainerFileHttpError::Client(PoolError::Closed)),
        "{error}"
    );
}

#[test]
fn delete_reports_the_transform_failure() {
    let mut fixture = Fixture::new();
    fixture.config.fail_transform = true;
    let call = fixture.call();
    let error = call_error(delete_error(&fixture, &call));
    assert!(
        matches!(
            error,
            ContainerFileHttpError::Config(ContainerError::ApiBaseRequired)
        ),
        "{error}"
    );
    assert!(fixture.source.seen().is_empty());
    assert!(fixture.hook.seen().is_empty());
}

#[rstest]
#[case::json_error(
    404,
    r#"{"error":{"message":"no such container"}}"#,
    "no such container"
)]
#[case::plain_text(503, "unavailable", "unavailable")]
fn delete_raises_the_response_error_before_decoding(
    #[case] status: u16,
    #[case] body: &str,
    #[case] message: &str,
) {
    let fixture = Fixture::new();
    fixture.source.answer(reply(status, body));
    let call = fixture.call();
    let (exception, synthesized) = provider_error(delete_error(&fixture, &call));
    assert!(!synthesized);
    assert_eq!(exception.status_code, status);
    assert_eq!(exception.message, message);
    let headers = header_map(&[("x-request-id", "req_1")]);
    assert_eq!(exception.headers, headers);
    assert_eq!(
        fixture.config.errors(),
        vec![ErrorSeen {
            message: message.to_owned(),
            status,
            headers,
        }]
    );
}

#[test]
fn delete_reports_a_body_that_does_not_decode() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, "not json"));
    let call = fixture.call();
    let error = call_error(delete_error(&fixture, &call));
    assert!(
        matches!(
            error,
            ContainerFileHttpError::Config(ContainerError::Decode(_))
        ),
        "{error}"
    );
}

#[test]
fn delete_reports_a_transport_failure_that_is_not_a_timeout_as_500() {
    let fixture = Fixture::new();
    fixture.source.fail(closed_error());
    let call = fixture.call();
    let (exception, synthesized) = provider_error(delete_error(&fixture, &call));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "the client is shutting down and takes no new requests"
    );
}

#[tokio::test(start_paused = true)]
async fn delete_reports_a_sync_timeout_as_408() {
    let fixture = Fixture::new();
    fixture.source.fail(timeout_error().await);
    let call = fixture.call();
    let (exception, synthesized) = provider_error(delete_error(&fixture, &call));
    assert!(!synthesized);
    assert_eq!(exception.status_code, 408);
    assert_eq!(exception.message, TIMEOUT_MESSAGE);
    assert_eq!(
        fixture.config.errors(),
        vec![ErrorSeen {
            message: TIMEOUT_MESSAGE.to_owned(),
            status: 408,
            headers: header_map(&[]),
        }]
    );
}

#[test]
fn delete_reports_a_rejected_timeout_as_408() {
    let fixture = Fixture::new();
    fixture.source.fail(rejected_timeout());
    let call = fixture.call();
    let (exception, synthesized) = provider_error(delete_error(&fixture, &call));
    assert!(!synthesized);
    assert_eq!(exception.status_code, 408);
    assert_eq!(exception.message, TIMEOUT_MESSAGE);
}

#[test]
fn delete_async_reports_a_rejected_timeout_as_a_synthesized_500() {
    let fixture = Fixture::new();
    fixture.source.fail(rejected_timeout());
    let handler = fixture.handler();
    let call = fixture.call();
    let (exception, synthesized) = provider_error(refused(
        runtime().block_on(handler.acontainer_delete_handler(&call)),
    ));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(exception.message, "the request timed out");
}

#[tokio::test(start_paused = true)]
async fn delete_async_reports_a_timeout_as_a_synthesized_500() {
    let fixture = Fixture::new();
    fixture.source.fail(timeout_error().await);
    let handler = fixture.handler();
    let call = fixture.call();
    let (exception, synthesized) =
        provider_error(refused(handler.acontainer_delete_handler(&call).await));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "the request ran past its read timeout or total deadline"
    );
}

#[test]
fn delete_async_sends_the_same_request_with_the_async_client() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, DELETED_JSON));
    let handler = fixture.handler();
    let call = fixture.call();
    let deleted = runtime()
        .block_on(handler.acontainer_delete_handler(&call))
        .expect("the delete succeeds");
    assert_eq!(deleted, deleted_container());
    assert_eq!(
        fixture.source.built(),
        vec![BuiltClient {
            asynchronous: true,
            ssl_verify: None,
        }]
    );
    let request = only(&fixture.source.seen());
    assert_eq!(request.method, ContainerMethod::Delete);
    assert_eq!(request.url, format!("{API_BASE}/containers/{CONTAINER_ID}"));
}

#[test]
fn delete_async_uses_the_supplied_async_client() {
    let fixture = Fixture::new();
    fixture.supplied.answer(reply(200, DELETED_JSON));
    let call = ContainerResourceCall {
        async_client: Some(&fixture.supplied),
        ..fixture.call()
    };
    let handler = fixture.handler();
    let deleted = runtime()
        .block_on(handler.acontainer_delete_handler(&call))
        .expect("the delete succeeds");
    assert_eq!(deleted, deleted_container());
    assert_eq!(fixture.supplied.seen().len(), 1);
    assert!(fixture.source.built().is_empty());
}

#[test]
fn delete_sync_call_with_the_async_flag_hands_back_the_coroutine() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, DELETED_JSON));
    let handler = fixture.handler();
    let call = fixture.call();
    let outcome = handler
        .container_delete_handler(&call, true)
        .expect("the handler returns");
    match outcome {
        Handled::Pending(pending) => {
            assert_eq!(
                runtime().block_on(pending).expect("the delete succeeds"),
                deleted_container()
            );
        }
        Handled::Done(_) => panic!("the async flag hands back a future"),
    }
    assert_eq!(
        fixture.source.built(),
        vec![BuiltClient {
            asynchronous: true,
            ssl_verify: None,
        }]
    );
    assert_eq!(only(&fixture.source.seen()).method, ContainerMethod::Delete);
}
