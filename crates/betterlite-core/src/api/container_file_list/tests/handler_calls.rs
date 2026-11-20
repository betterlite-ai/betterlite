#![cfg(test)]

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;
use indexmap::IndexMap;

use super::error::FileListHandlerError;
use super::execution::{container_file_list_handler, extra_query_object, query_pairs};
// ast-grep-ignore: no-long-names
use super::execution::async_container_file_list_handler;
use super::test_support::{
    CONTAINER_ID, FILES_BASE, FakeClient, FakeSource, Fixture, RecordingHook, object,
};

fn transformed_url() -> String {
    format!("{FILES_BASE}/{CONTAINER_ID}/files")
}

#[test]
fn the_blocking_handler_sends_the_transformed_request() {
    let fixture = Fixture::new();
    let params = fixture.params();

    let response = fixture.handle(&params).expect("the file list decodes");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, ContainerMethod::Get);
    assert_eq!(requests[0].url, transformed_url());
    assert!(requests[0].query.is_empty());
    assert_eq!(
        requests[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer None")
    );
}

#[test]
fn a_supplied_blocking_client_is_reused() {
    let fixture = Fixture::new();
    let params = fixture.params();
    let client = fixture.source.client.clone();

    let call = fixture.handler_call::<FakeSource>(&params, Some(&client), None);
    let response =
        container_file_list_handler(&fixture.source, &fixture.hook, &call).expect("the send");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(fixture.source.blocking_ssl().is_empty());
    assert_eq!(client.requests().len(), 1);
}

#[test]
fn a_new_blocking_client_is_built_with_ssl_verify() {
    let fixture = Fixture::new();
    let params = object(r#"{"ssl_verify":true}"#);

    fixture.handle(&params).expect("the file list decodes");

    assert_eq!(
        fixture.source.blocking_ssl(),
        vec![Some(SslVerify::Enabled(true))]
    );
}

#[test]
fn a_ca_bundle_reaches_the_client_builder() {
    let fixture = Fixture::new();
    let params = object(r#"{"ssl_verify":"/certs/ca.pem"}"#);

    fixture.handle(&params).expect("the file list decodes");

    assert_eq!(
        fixture.source.blocking_ssl(),
        vec![Some(SslVerify::CaBundle("/certs/ca.pem".to_owned()))]
    );
}

#[test]
fn an_absent_ssl_verify_builds_the_client_with_none() {
    let fixture = Fixture::new();
    let params = fixture.params();

    fixture.handle(&params).expect("the file list decodes");

    assert_eq!(fixture.source.blocking_ssl(), vec![None]);
}

#[tokio::test]
async fn a_supplied_async_client_is_reused() {
    let fixture = Fixture::new();
    let params = fixture.params();
    let client = fixture.source.client.clone();

    let call = fixture.handler_call::<FakeSource>(&params, None, Some(&client));
    // ast-grep-ignore: no-long-names
    let response = async_container_file_list_handler(&fixture.source, &fixture.hook, &call)
        .await
        .expect("the send");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(fixture.source.async_ssl().is_empty());
    assert_eq!(client.requests().len(), 1);
}

#[tokio::test]
async fn a_new_async_client_is_built_with_ssl_verify() {
    let fixture = Fixture::new();
    let params = object(r#"{"ssl_verify":"/certs/ca.pem"}"#);

    let call = fixture.handler_call::<FakeSource>(&params, None, None);
    // ast-grep-ignore: no-long-names
    async_container_file_list_handler(&fixture.source, &fixture.hook, &call)
        .await
        .expect("the file list decodes");

    assert_eq!(
        fixture.source.async_ssl(),
        vec![Some(SslVerify::CaBundle("/certs/ca.pem".to_owned()))]
    );
    assert!(fixture.source.blocking_ssl().is_empty());
}

#[test]
fn a_provider_error_status_is_a_provider_failure() {
    let fixture = Fixture::answering(404, r#"{"error":{"message":"missing"}}"#);
    let params = fixture.params();

    let error = fixture
        .handle(&params)
        .expect_err("the provider answered 404");

    let FileListHandlerError::Provider(provider) = &error else {
        panic!("expected a provider failure, got {error:?}");
    };
    assert_eq!(provider.status_code, 404);
    assert_eq!(provider.message, "missing");
}

#[test]
fn a_non_json_error_body_keeps_its_text() {
    let fixture = Fixture::answering(500, "boom");
    let params = fixture.params();

    let error = fixture
        .handle(&params)
        .expect_err("the provider answered 500");

    let FileListHandlerError::Provider(provider) = &error else {
        panic!("expected a provider failure, got {error:?}");
    };
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.message, "boom");
}

#[test]
fn a_transport_failure_is_the_transports_answer() {
    let mut fixture = Fixture::new();
    fixture.source.client = FakeClient::failing();
    let params = fixture.params();

    let error = fixture.handle(&params).expect_err("the client is closed");

    let FileListHandlerError::Transport(transport) = &error else {
        panic!("expected a transport failure, got {error:?}");
    };
    assert_eq!(transport.status_code, 500);
    assert_eq!(
        transport.message,
        "the client is shutting down and takes no new requests"
    );
}

#[test]
fn a_hook_failure_stops_the_send() {
    let mut fixture = Fixture::new();
    fixture.hook = RecordingHook::failing();
    let params = fixture.params();

    let error = fixture
        .handle(&params)
        .expect_err("the hook refuses the call");

    assert!(matches!(
        error,
        FileListHandlerError::Logging(PrePostError::RawRequestCaptured)
    ));
    assert!(fixture.requests().is_empty());
}

#[test]
fn a_wrong_typed_ssl_verify_is_refused() {
    let fixture = Fixture::new();
    let params = object(r#"{"ssl_verify":1}"#);

    let error = fixture
        .handle(&params)
        .expect_err("the ssl_verify is a number");

    let FileListHandlerError::Params(ContainerFileHttpError::ParamType { field, .. }) = &error
    else {
        panic!("expected a wrong-typed param, got {error:?}");
    };
    assert_eq!(*field, "ssl_verify");
    assert!(fixture.source.blocking_ssl().is_empty());
    assert!(fixture.requests().is_empty());
}

#[test]
fn a_wrong_typed_api_key_is_refused() {
    let fixture = Fixture::new();
    let params = object(r#"{"api_key":1}"#);

    let error = fixture
        .handle(&params)
        .expect_err("the api_key is a number");

    let FileListHandlerError::Params(ContainerFileHttpError::ParamType { field, .. }) = &error
    else {
        panic!("expected a wrong-typed param, got {error:?}");
    };
    assert_eq!(*field, "api_key");
}

#[test]
fn query_pairs_render_a_null_and_a_list_and_refuse_nested_values() {
    let params = object(r#"{"a":null,"b":true,"c":3,"d":["x","y"]}"#);

    let pairs = query_pairs(&params).expect("the single values render");

    assert_eq!(
        pairs,
        vec![
            ("a".to_owned(), String::new()),
            ("b".to_owned(), "true".to_owned()),
            ("c".to_owned(), "3".to_owned()),
            ("d".to_owned(), "x".to_owned()),
            ("d".to_owned(), "y".to_owned()),
        ]
    );
    let nested = object(r#"{"e":{"k":1}}"#);
    let error = query_pairs(&nested).expect_err("an object cannot be a query value");
    let FileListHandlerError::QueryValue { field } = &error else {
        panic!("expected a refused query value, got {error:?}");
    };
    assert_eq!(field, "e");
}

#[test]
fn extra_query_object_turns_param_values_into_json() {
    let query = IndexMap::from([
        ("number".to_owned(), ParamValue::Integer(3)),
        ("text".to_owned(), ParamValue::Text("v".to_owned())),
    ]);

    let encoded = extra_query_object(Some(&query)).expect("the query has values");

    assert_eq!(encoded, object(r#"{"number":3,"text":"v"}"#));
    assert!(extra_query_object(None).is_none());
}

#[test]
fn the_handler_call_timeout_is_not_part_of_the_request() {
    let fixture = Fixture::new();
    let params = fixture.params();
    let mut quick = fixture.handler_call::<FakeSource>(&params, None, None);
    quick.timeout_seconds = 5.0;
    container_file_list_handler(&fixture.source, &fixture.hook, &quick).expect("the send");
    let mut slow = fixture.handler_call::<FakeSource>(&params, None, None);
    slow.timeout_seconds = 600.0;
    container_file_list_handler(&fixture.source, &fixture.hook, &slow).expect("the send");

    let requests = fixture.requests();

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0].url, transformed_url());
    assert!(requests[0].query.is_empty());
}
