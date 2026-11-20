#![cfg(test)]

use rstest::rstest;

use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;

use super::tests::{
    API_BASE, BuiltClient, CONTAINER_ID, CONTAINER_JSON, ErrorSeen, FakeSource, Fixture, Logged,
    StubConfig, TransformSeen, call_error, closed_error, container_object, done, header_map,
    map_of, only, param_field, params, provider_error, query, refused, rejected_timeout, reply,
    runtime, timeout_error,
};
use super::{ContainerResourceCall, ContainerResourceHttpError, Handled};

fn run_retrieve(
    fixture: &Fixture,
    call: &ContainerResourceCall<'_, FakeSource>,
) -> ContainerObject {
    done(fixture.handler().container_retrieve_handler(call, false)).expect("the retrieve succeeds")
}

fn retrieve_error(
    fixture: &Fixture,
    call: &ContainerResourceCall<'_, FakeSource>,
) -> ContainerResourceHttpError {
    refused(fixture.handler().container_retrieve_handler(call, false))
}

#[test]
fn retrieve_sends_a_get_with_the_provider_headers() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    assert_eq!(run_retrieve(&fixture, &call), container_object());
    let request = only(&fixture.source.seen());
    assert_eq!(request.method, ContainerMethod::Get);
    assert_eq!(request.url, format!("{API_BASE}/containers/{CONTAINER_ID}"));
    let headers = header_map(&[
        ("authorization", "Bearer sk-test"),
        ("x-provider", "validated"),
    ]);
    assert_eq!(request.headers, headers);
    assert!(request.query.is_empty());
    assert_eq!(
        fixture.config.seen(),
        vec![TransformSeen {
            container_id: CONTAINER_ID.to_owned(),
            api_base: API_BASE.to_owned(),
            headers,
        }]
    );
    assert_eq!(
        fixture.config.urls(),
        vec![Some("https://base.example".to_owned())]
    );
    assert_eq!(fixture.config.keys(), vec![Some("sk-test".to_owned())]);
}

#[test]
fn retrieve_logs_the_transformed_call_before_sending() {
    let mut fixture = Fixture::new();
    fixture.config.params = params(r#"{"limit":"5","order":"asc"}"#);
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    assert_eq!(run_retrieve(&fixture, &call), container_object());
    assert_eq!(
        fixture.hook.seen(),
        vec![Logged {
            url: format!("{API_BASE}/containers/{CONTAINER_ID}"),
            headers: header_map(&[
                ("authorization", "Bearer sk-test"),
                ("x-provider", "validated"),
            ]),
            params: header_map(&[("limit", "5"), ("order", "asc")]),
            container_id: CONTAINER_ID.to_owned(),
        }]
    );
    assert_eq!(
        map_of(&only(&fixture.source.seen()).query),
        header_map(&[("limit", "5"), ("order", "asc")])
    );
}

#[test]
fn retrieve_lets_extra_query_override_the_transformed_query() {
    let mut fixture = Fixture::new();
    fixture.config.params = params(r#"{"after":"a","limit":"5"}"#);
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let extra = query(&[
        ("limit", ParamValue::Text("9".to_owned())),
        ("order", ParamValue::Text("desc".to_owned())),
    ]);
    let call = ContainerResourceCall {
        extra_query: Some(&extra),
        ..fixture.call()
    };
    run_retrieve(&fixture, &call);
    let expected = header_map(&[("after", "a"), ("limit", "9"), ("order", "desc")]);
    assert_eq!(map_of(&only(&fixture.source.seen()).query), expected);
    assert_eq!(
        fixture.hook.seen().first().expect("one record").params,
        expected
    );
}

#[test]
fn retrieve_lays_extra_headers_over_the_validation_answer() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let extra = header_map(&[("authorization", "Bearer extra"), ("x-extra", "yes")]);
    let call = ContainerResourceCall {
        extra_headers: Some(&extra),
        ..fixture.call()
    };
    run_retrieve(&fixture, &call);
    let request = only(&fixture.source.seen());
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer extra")
    );
    assert_eq!(
        request.headers.get("x-extra").map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        request.headers.get("x-provider").map(String::as_str),
        Some("validated")
    );
    assert_eq!(fixture.config.keys(), vec![Some("sk-test".to_owned())]);
}

#[test]
fn retrieve_renders_scalar_query_values() {
    let mut fixture = Fixture::new();
    fixture.config.params =
        params(r#"{"count":7,"empty":null,"flag":true,"name":"x","off":false,"ratio":1.5}"#);
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    run_retrieve(&fixture, &call);
    assert_eq!(
        map_of(&only(&fixture.source.seen()).query),
        header_map(&[
            ("count", "7"),
            ("empty", ""),
            ("flag", "true"),
            ("name", "x"),
            ("off", "false"),
            ("ratio", "1.5"),
        ])
    );
}

#[test]
fn retrieve_keeps_the_urls_own_query_and_carries_the_params() {
    let mut fixture = Fixture::new();
    fixture.config.transformed_url =
        Some("https://api.example/v1/containers/cntr_1?api-version=old".to_owned());
    fixture.config.params = params(r#"{"limit":"5"}"#);
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    run_retrieve(&fixture, &call);
    let request = only(&fixture.source.seen());
    assert_eq!(
        request.url,
        "https://api.example/v1/containers/cntr_1?api-version=old"
    );
    assert_eq!(map_of(&request.query), header_map(&[("limit", "5")]));
}

#[rstest]
#[case::list(r#"{"tags":["a"]}"#)]
#[case::object(r#"{"tags":{"a":1}}"#)]
fn retrieve_reports_a_query_value_that_is_not_scalar(#[case] transformed: &str) {
    let mut fixture = Fixture::new();
    fixture.config.params = params(transformed);
    let call = fixture.call();
    match retrieve_error(&fixture, &call) {
        ContainerResourceHttpError::QueryValue { name } => assert_eq!(name, "tags"),
        ContainerResourceHttpError::Call(error) => panic!("the call failed: {error}"),
        ContainerResourceHttpError::Provider { exception, .. } => {
            panic!("the provider failed: {exception}")
        }
    }
    assert!(fixture.source.seen().is_empty());
    assert!(fixture.hook.seen().is_empty());
}

#[rstest]
#[case::api_key(r#"{"api_key":5}"#, "api_key")]
#[case::ssl_verify(r#"{"ssl_verify":5}"#, "ssl_verify")]
fn retrieve_reports_a_param_the_handler_cannot_read(#[case] litellm: &str, #[case] field: &str) {
    let mut fixture = Fixture::new();
    fixture.litellm_params = params(litellm);
    let call = fixture.call();
    assert_eq!(param_field(retrieve_error(&fixture, &call)), field);
    assert!(fixture.source.seen().is_empty());
}

#[test]
fn retrieve_reports_the_base_url_the_config_refuses() {
    let mut fixture = Fixture::new();
    fixture.config = StubConfig::without_url();
    let call = fixture.call();
    let error = call_error(retrieve_error(&fixture, &call));
    assert!(
        matches!(
            error,
            ContainerFileHttpError::Config(ContainerError::ApiBaseRequired)
        ),
        "{error}"
    );
    assert!(fixture.source.seen().is_empty());
}

#[test]
fn retrieve_reports_the_transform_failure() {
    let mut fixture = Fixture::new();
    fixture.config.fail_transform = true;
    let call = fixture.call();
    let error = call_error(retrieve_error(&fixture, &call));
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

#[test]
fn retrieve_reports_a_client_the_source_refuses() {
    let fixture = Fixture::new();
    fixture.source.refuse();
    let call = fixture.call();
    let error = call_error(retrieve_error(&fixture, &call));
    assert!(
        matches!(error, ContainerFileHttpError::Client(PoolError::Closed)),
        "{error}"
    );
}

#[test]
fn retrieve_uses_the_supplied_blocking_client() {
    let fixture = Fixture::new();
    fixture.supplied.answer(reply(200, CONTAINER_JSON));
    let call = ContainerResourceCall {
        blocking_client: Some(&fixture.supplied),
        ..fixture.call()
    };
    assert_eq!(run_retrieve(&fixture, &call), container_object());
    assert_eq!(fixture.supplied.seen().len(), 1);
    assert!(fixture.source.built().is_empty());
    assert!(fixture.source.seen().is_empty());
}

#[rstest]
#[case::enabled(r#"{"api_key":"sk-test","ssl_verify":true}"#, SslVerify::Enabled(true))]
#[case::bundle(
    r#"{"api_key":"sk-test","ssl_verify":"ca.pem"}"#,
    SslVerify::CaBundle("ca.pem".to_owned())
)]
fn retrieve_builds_a_client_with_the_params_ssl_verify(
    #[case] litellm: &str,
    #[case] expected: SslVerify,
) {
    let mut fixture = Fixture::new();
    fixture.litellm_params = params(litellm);
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    run_retrieve(&fixture, &call);
    assert_eq!(
        fixture.source.built(),
        vec![BuiltClient {
            asynchronous: false,
            ssl_verify: Some(expected),
        }]
    );
}

#[rstest]
#[case::json_error(400, r#"{"error":{"message":"bad container"}}"#, "bad container")]
#[case::plain_text(500, "boom", "boom")]
fn retrieve_raises_the_response_error_before_decoding(
    #[case] status: u16,
    #[case] body: &str,
    #[case] message: &str,
) {
    let fixture = Fixture::new();
    fixture.source.answer(reply(status, body));
    let call = fixture.call();
    let (exception, synthesized) = provider_error(retrieve_error(&fixture, &call));
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

#[rstest]
#[case::ok(200)]
#[case::redirect(399)]
#[case::unknown(600)]
fn retrieve_decodes_a_status_that_is_not_an_error(#[case] status: u16) {
    let fixture = Fixture::new();
    fixture.source.answer(reply(status, CONTAINER_JSON));
    let call = fixture.call();
    assert_eq!(run_retrieve(&fixture, &call), container_object());
    assert!(fixture.config.errors().is_empty());
}

#[test]
fn retrieve_reports_a_body_that_does_not_decode() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, "not json"));
    let call = fixture.call();
    let error = call_error(retrieve_error(&fixture, &call));
    assert!(
        matches!(
            error,
            ContainerFileHttpError::Config(ContainerError::Decode(_))
        ),
        "{error}"
    );
}

#[test]
fn retrieve_reports_a_transport_failure_as_a_synthesized_500() {
    let fixture = Fixture::new();
    fixture.source.fail(closed_error());
    let call = fixture.call();
    let (exception, synthesized) = provider_error(retrieve_error(&fixture, &call));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "the client is shutting down and takes no new requests"
    );
    assert!(exception.headers.is_empty());
    assert_eq!(
        fixture.config.errors(),
        vec![ErrorSeen {
            message: exception.message,
            status: 500,
            headers: header_map(&[]),
        }]
    );
}

#[test]
fn retrieve_reports_a_rejected_timeout_as_a_synthesized_500() {
    let fixture = Fixture::new();
    fixture.source.fail(rejected_timeout());
    let call = fixture.call();
    let (exception, synthesized) = provider_error(retrieve_error(&fixture, &call));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(exception.message, "the request timed out");
}

#[tokio::test(start_paused = true)]
async fn retrieve_reports_a_timeout_as_a_synthesized_500() {
    let fixture = Fixture::new();
    fixture.source.fail(timeout_error().await);
    let call = fixture.call();
    let (exception, synthesized) = provider_error(retrieve_error(&fixture, &call));
    assert!(synthesized);
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "the request ran past its read timeout or total deadline"
    );
}

#[test]
fn retrieve_stops_before_sending_when_the_hook_fails() {
    let fixture = Fixture::new();
    fixture.hook.refuse();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = fixture.call();
    match call_error(retrieve_error(&fixture, &call)) {
        ContainerFileHttpError::Logging(PrePostError::CallbackFailed { message }) => {
            assert_eq!(message, "the hook refused the call");
        }
        error => panic!("the call failed: {error}"),
    }
    assert_eq!(fixture.hook.seen().len(), 1);
    assert!(fixture.source.seen().is_empty());
}

#[test]
fn retrieve_async_sends_the_same_request_with_the_async_client() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let handler = fixture.handler();
    let call = fixture.call();
    let container = runtime()
        .block_on(handler.acontainer_retrieve_handler(&call))
        .expect("the retrieve succeeds");
    assert_eq!(container, container_object());
    assert_eq!(
        fixture.source.built(),
        vec![BuiltClient {
            asynchronous: true,
            ssl_verify: None,
        }]
    );
    let request = only(&fixture.source.seen());
    assert_eq!(request.method, ContainerMethod::Get);
    assert_eq!(request.url, format!("{API_BASE}/containers/{CONTAINER_ID}"));
}

#[test]
fn retrieve_async_uses_the_supplied_async_client() {
    let fixture = Fixture::new();
    fixture.supplied.answer(reply(200, CONTAINER_JSON));
    let call = ContainerResourceCall {
        async_client: Some(&fixture.supplied),
        ..fixture.call()
    };
    let handler = fixture.handler();
    let container = runtime()
        .block_on(handler.acontainer_retrieve_handler(&call))
        .expect("the retrieve succeeds");
    assert_eq!(container, container_object());
    assert_eq!(fixture.supplied.seen().len(), 1);
    assert!(fixture.source.built().is_empty());
}

#[test]
fn retrieve_async_builds_an_async_client_when_only_a_blocking_one_is_supplied() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let call = ContainerResourceCall {
        blocking_client: Some(&fixture.supplied),
        ..fixture.call()
    };
    let handler = fixture.handler();
    runtime()
        .block_on(handler.acontainer_retrieve_handler(&call))
        .expect("the retrieve succeeds");
    assert!(fixture.supplied.seen().is_empty());
    assert_eq!(
        fixture.source.built(),
        vec![BuiltClient {
            asynchronous: true,
            ssl_verify: None,
        }]
    );
    assert_eq!(fixture.source.seen().len(), 1);
}

#[test]
fn retrieve_sync_call_with_the_async_flag_hands_back_the_coroutine() {
    let fixture = Fixture::new();
    fixture.source.answer(reply(200, CONTAINER_JSON));
    let handler = fixture.handler();
    let call = fixture.call();
    let outcome = handler
        .container_retrieve_handler(&call, true)
        .expect("the handler returns");
    match outcome {
        Handled::Pending(pending) => {
            assert_eq!(
                runtime().block_on(pending).expect("the retrieve succeeds"),
                container_object()
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
    assert_eq!(only(&fixture.source.seen()).method, ContainerMethod::Get);
}

#[test]
fn retrieve_sync_call_with_the_async_flag_reads_the_params_only_when_polled() {
    let mut fixture = Fixture::new();
    fixture.litellm_params = params(r#"{"api_key":5}"#);
    let handler = fixture.handler();
    let call = fixture.call();
    let outcome = handler
        .container_retrieve_handler(&call, true)
        .expect("the handler returns before reading the params");
    match outcome {
        Handled::Pending(pending) => {
            assert_eq!(param_field(refused(runtime().block_on(pending))), "api_key");
        }
        Handled::Done(_) => panic!("the async flag hands back a future"),
    }
    assert!(fixture.source.seen().is_empty());
}
