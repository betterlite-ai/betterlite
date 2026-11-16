#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::ContainerFileReply;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::json;

use super::support::{
    HandlerForm, StubReply, TIMEOUT_SECONDS, credentialed_list_call, deleted_file, file_list,
    file_object, fixture, list_call, logging, managed_retrieve_call, retrieve_call, upload,
    upload_call,
};

#[test]
fn handler_receives_the_endpoint_config_and_params() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let call = credentialed_list_call();
    endpoints
        .container_file_call("list_container_files", &call, &logging(), TIMEOUT_SECONDS)
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].endpoint_name, "list_container_files");
    assert_eq!(recorded[0].config_label, "openai");
    let values = BTreeMap::from([
        (
            "container_id".to_owned(),
            ParamValue::Text("cntr_1".to_owned()),
        ),
        ("file_id".to_owned(), ParamValue::Text("cfile_7".to_owned())),
        ("after".to_owned(), ParamValue::Text("cfile_0".to_owned())),
        ("limit".to_owned(), ParamValue::Integer(10)),
        ("order".to_owned(), ParamValue::Text("asc".to_owned())),
    ]);
    assert_eq!(recorded[0].values, values);
    assert_eq!(recorded[0].params.get("api_key"), Some(&json!("sk-test")));
    assert_eq!(
        recorded[0].params.get("api_base"),
        Some(&json!("https://api.example"))
    );
    assert_eq!(
        recorded[0].params.get("api_version"),
        Some(&json!("2024-01-01"))
    );
    assert_eq!(recorded[0].params.get("model"), Some(&json!("gpt-4o-mini")));
    assert_eq!(recorded[0].params.get("container_id"), None);
    assert_eq!(recorded[0].file, None);
}

#[test]
fn handler_receives_the_caller_headers_query_and_timeout() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.extra_headers = Some(BTreeMap::from([("x-extra".to_owned(), "yes".to_owned())]));
    call.extra_query = Some(IndexMap::from([(
        "trace".to_owned(),
        ParamValue::Text("t-1".to_owned()),
    )]));
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].headers, call.extra_headers);
    assert_eq!(recorded[0].query, call.extra_query);
    assert_eq!(recorded[0].timeout, Duration::from_secs(600));
    assert_eq!(recorded[0].form, HandlerForm::Blocking);
}

#[test]
fn handler_receives_the_decoded_container_id() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &managed_retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(
        recorded[0].values.get("container_id"),
        Some(&ParamValue::Text("cntr_1".to_owned()))
    );
    assert_eq!(fixture.decoder.seen()[0].0, "litellm-managed-abc");
}

#[test]
fn handler_receives_the_logging_state() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].call_id, "call-1");
    let optional_params = IndexMap::from([
        ("container_id".to_owned(), Json::Text("cntr_1".to_owned())),
        ("file_id".to_owned(), Json::Text("cfile_1".to_owned())),
    ]);
    assert_eq!(recorded[0].optional_params, optional_params);
}

#[test]
fn handler_receives_the_upload() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "upload_container_file",
            &upload_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].file, Some(upload()));
    assert_eq!(
        recorded[0].values.get("container_id"),
        Some(&ParamValue::Text("cntr_1".to_owned()))
    );
}

#[test]
fn handler_returns_a_list_reply() {
    let mut fixture = fixture();
    fixture.handler.reply = StubReply::List(file_list());
    let endpoints = fixture.endpoints();
    let reply = endpoints
        .container_file_call(
            "list_container_files",
            &list_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::List(file_list()));
}

#[test]
fn handler_returns_a_file_reply() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let reply = endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::File(file_object()));
}

#[test]
fn handler_returns_a_deleted_reply() {
    let mut fixture = fixture();
    fixture.handler.reply = StubReply::Deleted(deleted_file());
    let endpoints = fixture.endpoints();
    let reply = endpoints
        .container_file_call(
            "delete_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::Deleted(deleted_file()));
}

#[test]
fn handler_returns_bytes_reply() {
    let mut fixture = fixture();
    let body = b"name,score\nalice,1\n".to_vec();
    fixture.handler.reply = StubReply::Bytes(body.clone());
    let endpoints = fixture.endpoints();
    let reply = endpoints
        .container_file_call(
            "retrieve_container_file_content",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(reply, ContainerFileReply::Bytes(body));
}

#[rstest]
#[case::absent(None, Duration::from_secs(6000))]
#[case::zero(Some(Duration::ZERO), Duration::from_secs(6000))]
#[case::given(Some(Duration::from_millis(1500)), Duration::from_millis(1500))]
fn handler_timeout_falls_back_to_the_default_request_timeout(
    #[case] timeout: Option<Duration>,
    #[case] expected: Duration,
) {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    call.timeout = timeout;
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].timeout, expected);
}

#[test]
fn handler_default_timeout_is_six_hundred_seconds() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    assert_eq!(fixture.handler.seen()[0].timeout, Duration::from_secs(600));
}

#[test]
fn handler_never_forwards_extra_body() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let mut call = retrieve_call();
    let mut extra_body = Object::new();
    extra_body.insert("ignored".to_owned(), json!(true));
    call.extra_body = Some(extra_body);
    endpoints
        .container_file_call(
            "retrieve_container_file",
            &call,
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the handler answers");
    let recorded = fixture.handler.seen();
    assert_eq!(recorded[0].params.get("extra_body"), None);
    assert!(recorded[0].params.get("ignored").is_none());
}

#[test]
fn handler_receives_the_catalog_name_for_an_async_name_call() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    endpoints
        .container_file_call(
            "aretrieve_container_file",
            &retrieve_call(),
            &logging(),
            TIMEOUT_SECONDS,
        )
        .expect("the async name resolves");
    assert_eq!(
        fixture.handler.seen()[0].endpoint_name,
        "retrieve_container_file"
    );
}
