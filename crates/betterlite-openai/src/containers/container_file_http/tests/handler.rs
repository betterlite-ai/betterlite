#![cfg(test)]

use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;

use super::support::{BASE, DELETE_BODY, FILE_BODY, FakeClient, LIST_BODY, fixture, handler};
use crate::containers::container_file_http::request::{ParamValue, SslVerify};
use crate::containers::container_file_http::transport::ContainerMethod;
use crate::containers::container_file_http::{ContainerFileHttpError, ContainerFileReply};

#[test]
fn handle_lists_files_through_a_blocking_client_built_for_the_call() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture
        .values
        .insert("after".to_owned(), ParamValue::Text("cfile_0".to_owned()));
    fixture
        .values
        .insert("limit".to_owned(), ParamValue::Integer(5));
    let reply = handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    assert!(matches!(reply, ContainerFileReply::List(list) if list.data.is_empty()));
    let requests = handler.clients.client.seen();
    let [request] = requests.as_slice() else {
        panic!("expected one request, got {requests:?}");
    };
    assert_eq!(request.method, ContainerMethod::Get);
    assert_eq!(request.url, format!("{BASE}/cntr_1/files"));
    assert_eq!(
        request.query,
        [
            ("after".to_owned(), "cfile_0".to_owned()),
            ("limit".to_owned(), "5".to_owned())
        ]
    );
    assert_eq!(handler.clients.blocking_made.lock().unwrap().len(), 1);
    assert!(handler.clients.async_made.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ahandle_lists_files_through_an_async_client_built_for_the_call() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let reply = handler
        .ahandle(&fixture.call("alist_container_files"))
        .await
        .unwrap();
    assert!(matches!(reply, ContainerFileReply::List(_)));
    assert_eq!(handler.clients.client.seen().len(), 1);
    assert_eq!(handler.clients.async_made.lock().unwrap().len(), 1);
    assert!(handler.clients.blocking_made.lock().unwrap().is_empty());
}

#[rstest]
#[case::list(
    "list_container_files",
    ContainerMethod::Get,
    "/cntr_1/files",
    LIST_BODY
)]
#[case::list_async_name(
    "alist_container_files",
    ContainerMethod::Get,
    "/cntr_1/files",
    LIST_BODY
)]
#[case::upload(
    "upload_container_file",
    ContainerMethod::Post,
    "/cntr_1/files",
    FILE_BODY
)]
#[case::retrieve(
    "retrieve_container_file",
    ContainerMethod::Get,
    "/cntr_1/files/cfile_1",
    FILE_BODY
)]
#[case::delete(
    "delete_container_file",
    ContainerMethod::Delete,
    "/cntr_1/files/cfile_1",
    DELETE_BODY
)]
#[case::content(
    "retrieve_container_file_content",
    ContainerMethod::Get,
    "/cntr_1/files/cfile_1/content",
    "raw bytes"
)]
fn handle_sends_each_catalog_endpoint_with_its_method_and_path(
    #[case] name: &str,
    #[case] method: ContainerMethod,
    #[case] path: &str,
    #[case] body: &str,
) {
    let handler = handler(200, body);
    let fixture = fixture();
    handler.handle(&fixture.call(name)).unwrap();
    let requests = handler.clients.client.seen();
    let [request] = requests.as_slice() else {
        panic!("expected one request, got {requests:?}");
    };
    assert_eq!(request.method, method);
    assert_eq!(request.url, format!("{BASE}{path}"));
}

#[rstest]
#[case::empty("")]
#[case::unknown("list_container")]
#[case::wrong_case("List_Container_Files")]
fn handle_rejects_an_unknown_endpoint_before_any_client_or_log(#[case] name: &str) {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let error = handler.handle(&fixture.call(name)).unwrap_err();
    assert!(
        matches!(&error, ContainerFileHttpError::UnknownEndpoint { name: reported } if reported == name)
    );
    assert_eq!(error.to_string(), format!("Unknown endpoint: {name}"));
    assert!(handler.clients.blocking_made.lock().unwrap().is_empty());
    assert!(handler.logging.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ahandle_rejects_an_unknown_endpoint_before_any_client_or_log() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let error = handler.ahandle(&fixture.call("nope")).await.unwrap_err();
    assert!(matches!(
        error,
        ContainerFileHttpError::UnknownEndpoint { .. }
    ));
    assert!(handler.clients.async_made.lock().unwrap().is_empty());
    assert!(handler.logging.seen.lock().unwrap().is_empty());
}

#[test]
fn handle_reuses_a_supplied_blocking_client() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let supplied = FakeClient::answering(200, LIST_BODY.as_bytes());
    let mut call = fixture.call("list_container_files");
    call.blocking_client = Some(&supplied);
    handler.handle(&call).unwrap();
    assert_eq!(supplied.seen().len(), 1);
    assert!(handler.clients.client.seen().is_empty());
    assert!(handler.clients.blocking_made.lock().unwrap().is_empty());
}

#[test]
fn handle_replaces_a_supplied_async_client_with_a_blocking_one() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let supplied = FakeClient::answering(200, LIST_BODY.as_bytes());
    let mut call = fixture.call("list_container_files");
    call.async_client = Some(&supplied);
    handler.handle(&call).unwrap();
    assert!(supplied.seen().is_empty());
    assert_eq!(handler.clients.client.seen().len(), 1);
    assert_eq!(handler.clients.blocking_made.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn ahandle_reuses_a_supplied_async_client() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let supplied = FakeClient::answering(200, LIST_BODY.as_bytes());
    let mut call = fixture.call("alist_container_files");
    call.async_client = Some(&supplied);
    handler.ahandle(&call).await.unwrap();
    assert_eq!(supplied.seen().len(), 1);
    assert!(handler.clients.client.seen().is_empty());
    assert!(handler.clients.async_made.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ahandle_replaces_a_supplied_blocking_client_with_an_async_one() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let supplied = FakeClient::answering(200, LIST_BODY.as_bytes());
    let mut call = fixture.call("alist_container_files");
    call.blocking_client = Some(&supplied);
    handler.ahandle(&call).await.unwrap();
    assert!(supplied.seen().is_empty());
    assert_eq!(handler.clients.client.seen().len(), 1);
    assert_eq!(handler.clients.async_made.lock().unwrap().len(), 1);
}

#[rstest]
#[case::absent(None, None)]
#[case::null(Some(Value::Null), None)]
#[case::disabled(Some(Value::Bool(false)), Some(SslVerify::Enabled(false)))]
#[case::enabled(Some(Value::Bool(true)), Some(SslVerify::Enabled(true)))]
#[case::bundle(Some(Value::String("/etc/ca.pem".to_owned())), Some(SslVerify::CaBundle("/etc/ca.pem".to_owned())))]
fn handle_passes_ssl_verify_to_the_client_it_creates(
    #[case] value: Option<Value>,
    #[case] expected: Option<SslVerify>,
) {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    if let Some(given) = value {
        fixture.params.insert("ssl_verify".to_owned(), given);
    }
    handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    assert_eq!(*handler.clients.blocking_made.lock().unwrap(), [expected]);
}

#[tokio::test]
async fn ahandle_passes_ssl_verify_to_the_client_it_creates() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture
        .params
        .insert("ssl_verify".to_owned(), Value::Bool(false));
    handler
        .ahandle(&fixture.call("alist_container_files"))
        .await
        .unwrap();
    assert_eq!(
        *handler.clients.async_made.lock().unwrap(),
        [Some(SslVerify::Enabled(false))]
    );
}

#[test]
fn handle_ignores_the_timeout_the_caller_passes() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    let mut call = fixture.call("list_container_files");
    handler.handle(&call).unwrap();
    call.timeout = Some(Duration::from_millis(1));
    handler.handle(&call).unwrap();
    let requests = handler.clients.client.seen();
    let [first, second] = requests.as_slice() else {
        panic!("expected two requests, got {requests:?}");
    };
    assert_eq!(first, second);
}

#[test]
fn handle_logs_the_final_url_headers_and_query_before_it_sends() {
    let handler = handler(200, LIST_BODY);
    let mut fixture = fixture();
    fixture
        .values
        .insert("order".to_owned(), ParamValue::Text("desc".to_owned()));
    handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    let seen = handler.logging.seen.lock().unwrap().clone();
    let [(input, args)] = seen.as_slice() else {
        panic!("expected one pre_call, got {}", seen.len());
    };
    assert_eq!(*input, Json::Text(String::new()));
    assert_eq!(
        args.api_base.as_deref(),
        Some(format!("{BASE}/cntr_1/files").as_str())
    );
    let headers = args.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text("Bearer none".to_owned()))
    );
    let Some(Json::Object(params)) = args.passthrough.get("params") else {
        panic!("params were not logged");
    };
    assert_eq!(params.get("order"), Some(&Json::Text("desc".to_owned())));
    assert_eq!(params.len(), 1);
}

#[test]
fn handle_logs_an_empty_params_object_when_no_query_is_set() {
    let handler = handler(200, LIST_BODY);
    let fixture = fixture();
    handler
        .handle(&fixture.call("list_container_files"))
        .unwrap();
    let seen = handler.logging.seen.lock().unwrap().clone();
    let [(_, args)] = seen.as_slice() else {
        panic!("expected one pre_call");
    };
    assert_eq!(
        args.passthrough.get("params"),
        Some(&Json::Object(IndexMap::new()))
    );
}

#[test]
fn handle_sends_nothing_when_the_logging_hook_fails() {
    let mut handler = handler(200, LIST_BODY);
    handler.logging.fail = true;
    let fixture = fixture();
    let error = handler
        .handle(&fixture.call("list_container_files"))
        .unwrap_err();
    assert!(matches!(error, ContainerFileHttpError::Logging(_)));
    assert!(handler.clients.client.seen().is_empty());
}
