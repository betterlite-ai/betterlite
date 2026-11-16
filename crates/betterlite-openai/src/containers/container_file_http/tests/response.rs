#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::Value;

use super::support::{BASE, DELETE_BODY, FILE_BODY, LIST_BODY, StubConfig, fixture, handler};
use crate::api::container_contracts::catalog::{ContainerEndpoint, container_endpoints};
use crate::containers::container_file_http::response::{
    error_message_from_response, raise_for_error_status, transform_reply,
};
use crate::containers::container_file_http::transport::ContainerHttpReply;
use crate::containers::container_file_http::{ContainerFileHttpError, ContainerFileReply};

fn endpoint(name: &str) -> ContainerEndpoint {
    container_endpoints()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.name == name)
        .unwrap()
}

fn reply(status: u16, body: &str) -> ContainerHttpReply {
    ContainerHttpReply {
        status,
        headers: BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

#[rstest]
#[case::json_message(r#"{"error":{"message":"no such file","type":"x"}}"#, "no such file")]
#[case::error_is_text(r#"{"error":"bad"}"#, r#"{"error":"bad"}"#)]
#[case::message_is_number(r#"{"error":{"message":4}}"#, r#"{"error":{"message":4}}"#)]
#[case::no_error_key(r#"{"detail":"x"}"#, r#"{"detail":"x"}"#)]
#[case::not_json("upstream gateway down", "upstream gateway down")]
#[case::json_list("[1,2]", "[1,2]")]
#[case::empty("", "")]
fn error_message_reads_the_json_error_message_or_the_body_text(
    #[case] body: &str,
    #[case] expected: &str,
) {
    assert_eq!(error_message_from_response(body.as_bytes()), expected);
}

#[test]
fn error_message_names_a_body_that_is_not_utf8_by_its_size() {
    assert_eq!(
        error_message_from_response(&[0x61, 0xff, 0x62]),
        "3 bytes that are not UTF-8"
    );
}

#[rstest]
#[case::ok(200, false)]
#[case::created(201, false)]
#[case::redirect(399, false)]
#[case::bad_request(400, true)]
#[case::not_found(404, true)]
#[case::server_error(500, true)]
#[case::last_error(599, true)]
#[case::beyond(600, false)]
fn raise_for_error_status_raises_only_for_4xx_and_5xx(#[case] status: u16, #[case] raises: bool) {
    let config = StubConfig::with_base(BASE);
    let outcome = raise_for_error_status(&config, &reply(status, r#"{"error":{"message":"m"}}"#));
    match (outcome, raises) {
        (Err(error), true) => {
            assert_eq!(error.status_code, status);
            assert_eq!(error.message, "m");
            assert_eq!(
                error.headers.get("x-request-id").map(String::as_str),
                Some("req_1")
            );
        }
        (Ok(()), false) => {}
        (other, _) => panic!("status {status}: {other:?}"),
    }
}

#[test]
fn handle_raises_the_provider_error_with_the_extracted_message() {
    let handler = handler(404, r#"{"error":{"message":"container not found"}}"#);
    let fixture = fixture();
    let error = handler
        .handle(&fixture.call("retrieve_container_file"))
        .unwrap_err();
    let ContainerFileHttpError::Provider(provider) = error else {
        panic!("expected a provider error, got {error:?}");
    };
    assert_eq!(provider.status_code, 404);
    assert_eq!(provider.message, "container not found");
}

#[tokio::test]
async fn ahandle_raises_the_provider_error_with_the_body_text() {
    let handler = handler(502, "bad gateway");
    let fixture = fixture();
    let error = handler
        .ahandle(&fixture.call("aretrieve_container_file"))
        .await
        .unwrap_err();
    let ContainerFileHttpError::Provider(provider) = error else {
        panic!("expected a provider error, got {error:?}");
    };
    assert_eq!(provider.status_code, 502);
    assert_eq!(provider.message, "bad gateway");
}

#[test]
fn handle_raises_for_an_error_field_on_a_successful_status() {
    let handler = handler(200, r#"{"error":{"message":"quota exceeded"}}"#);
    let fixture = fixture();
    let error = handler
        .handle(&fixture.call("retrieve_container_file"))
        .unwrap_err();
    let ContainerFileHttpError::Provider(provider) = error else {
        panic!("expected a provider error, got {error:?}");
    };
    assert_eq!(provider.status_code, 200);
    assert_eq!(provider.message, "quota exceeded");
}

#[rstest]
#[case::no_message(r#"{"error":{"code":"x"}}"#)]
#[case::text_error(r#"{"error":"boom"}"#)]
#[case::null_error(r#"{"error":null}"#)]
fn handle_reports_the_whole_body_when_a_successful_error_has_no_message(#[case] body: &str) {
    let handler = handler(200, body);
    let fixture = fixture();
    let error = handler
        .handle(&fixture.call("list_container_files"))
        .unwrap_err();
    let ContainerFileHttpError::Provider(provider) = error else {
        panic!("expected a provider error, got {error:?}");
    };
    let sent: Value = serde_json::from_str(body).unwrap();
    let reported: Value = serde_json::from_str(&provider.message).unwrap();
    assert_eq!(reported, sent);
}

#[test]
fn transform_returns_binary_content_untouched_even_when_it_looks_like_json() {
    let config = StubConfig::with_base(BASE);
    let answer = reply(200, r#"{"error":"not an error here"}"#);
    let result = transform_reply(
        &config,
        &endpoint("retrieve_container_file_content"),
        &answer,
    )
    .unwrap();
    assert_eq!(result, ContainerFileReply::Bytes(answer.body));
}

#[test]
fn transform_raises_for_an_error_status_on_a_binary_endpoint() {
    let config = StubConfig::with_base(BASE);
    let error = transform_reply(
        &config,
        &endpoint("retrieve_container_file_content"),
        &reply(404, "gone"),
    )
    .unwrap_err();
    assert!(
        matches!(error, ContainerFileHttpError::Provider(provider) if provider.message == "gone")
    );
}

#[rstest]
#[case::list("list_container_files", LIST_BODY)]
#[case::upload("upload_container_file", FILE_BODY)]
#[case::retrieve("retrieve_container_file", FILE_BODY)]
#[case::delete("delete_container_file", DELETE_BODY)]
fn transform_validates_the_body_against_the_response_type(#[case] name: &str, #[case] body: &str) {
    let config = StubConfig::with_base(BASE);
    let result = transform_reply(&config, &endpoint(name), &reply(200, body)).unwrap();
    match (name, result) {
        ("list_container_files", ContainerFileReply::List(list)) => assert!(!list.has_more),
        ("upload_container_file" | "retrieve_container_file", ContainerFileReply::File(file)) => {
            assert_eq!((file.id.as_str(), file.bytes), ("cfile_1", Some(12)));
        }
        ("delete_container_file", ContainerFileReply::Deleted(deleted)) => assert!(deleted.deleted),
        (other, outcome) => panic!("{other}: {outcome:?}"),
    }
}

#[test]
fn transform_returns_the_json_as_it_is_for_a_response_type_without_a_model() {
    let config = StubConfig::with_base(BASE);
    let mut raw = endpoint("retrieve_container_file");
    raw.response_type = "raw".to_owned();
    let result = transform_reply(&config, &raw, &reply(200, r#"{"a":[1,2],"b":null}"#)).unwrap();
    assert_eq!(
        result,
        ContainerFileReply::Raw(serde_json::from_str(r#"{"a":[1,2],"b":null}"#).unwrap())
    );
}

#[test]
fn transform_rejects_a_body_that_does_not_fit_the_model() {
    let config = StubConfig::with_base(BASE);
    let error = transform_reply(
        &config,
        &endpoint("retrieve_container_file"),
        &reply(200, r#"{"id":"x"}"#),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ContainerFileHttpError::Model { response_type, .. } if response_type == "ContainerFileObject"),
        "{error:?}"
    );
}

#[test]
fn transform_rejects_a_body_that_is_not_json() {
    let config = StubConfig::with_base(BASE);
    let error = transform_reply(
        &config,
        &endpoint("retrieve_container_file"),
        &reply(200, "<html>"),
    )
    .unwrap_err();
    assert!(matches!(error, ContainerFileHttpError::Json { .. }));
}

#[test]
fn transform_does_not_treat_a_redirect_status_as_an_error() {
    let config = StubConfig::with_base(BASE);
    let result = transform_reply(
        &config,
        &endpoint("retrieve_container_file"),
        &reply(302, FILE_BODY),
    )
    .unwrap();
    assert!(matches!(result, ContainerFileReply::File(_)));
}
