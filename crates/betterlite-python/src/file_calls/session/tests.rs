#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;

use super::NativeFileCallSession;
use crate::file_calls::error::NativeFileCallError;

const ENVIRONMENT: &str = r#"{"environment": {}}"#;

const CREATED: &str = r#"{"id":"file-abc","object":"file","bytes":4,"created_at":1700000000,"filename":"x.jsonl","purpose":"batch","status":"processed"}"#;
const DELETED: &str = r#"{"id":"file-abc","object":"file","deleted":true}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"file-abc","object":"file","bytes":4,"created_at":1700000000,"filename":"x.jsonl","purpose":"batch"}],"has_more":false,"first_id":"file-abc","last_id":"file-abc"}"#;

fn create_payload(address: &SocketAddr) -> String {
    format!(
        r#"{{"file":{{"$file":{{"name":"x.jsonl","blob":0,"content_type":"application/json"}}}},"purpose":"batch","custom_llm_provider":"openai","params":{{"api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

fn delete_payload(address: &SocketAddr) -> String {
    format!(
        r#"{{"file_id":"file-abc","custom_llm_provider":"openai","params":{{"api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

fn retrieve_payload(address: &SocketAddr) -> String {
    format!(
        r#"{{"file_id":"file-abc","custom_llm_provider":"openai","params":{{"api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

fn list_payload(address: &SocketAddr) -> String {
    format!(
        r#"{{"purpose":"batch","custom_llm_provider":"openai","params":{{"api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

#[test]
fn a_malformed_settings_text_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let error = NativeFileCallSession::new("not a session").err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn a_malformed_payload_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let create_error = session.create(py, "not a call", Vec::new()).err().unwrap();
        assert!(create_error.is_instance_of::<PyValueError>(py));
        let delete_error = session.delete(py, "not a call", Vec::new()).err().unwrap();
        assert!(delete_error.is_instance_of::<PyValueError>(py));
        let retrieve_error = session
            .retrieve(py, "not a call", Vec::new())
            .err()
            .unwrap();
        assert!(retrieve_error.is_instance_of::<PyValueError>(py));
        let list_error = session.list(py, "not a call", Vec::new()).err().unwrap();
        assert!(list_error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn a_create_call_posts_the_file_and_hands_out_the_object() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], CREATED),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let text = session
            .create(py, &create_payload(&socket), vec![b"payload".to_vec()])
            .unwrap();
        assert!(text.contains(r#""id":"file-abc""#));
        assert!(text.contains(r#""object":"file""#));
        assert!(text.contains(r#""status":"processed""#));
    });
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with("POST /v1/files"));
    assert!(request.body.contains(r#"name="file""#));
    assert!(request.body.contains(r#"filename="x.jsonl""#));
    assert!(request.body.contains("Content-Type: application/json"));
    assert!(request.body.contains("payload"));
    assert!(request.body.contains("batch"));
}

#[test]
fn a_delete_call_deletes_the_file_and_hands_out_the_flag() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], DELETED),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let text = session
            .delete(py, &delete_payload(&socket), Vec::new())
            .unwrap();
        assert!(text.contains(r#""id":"file-abc""#));
        assert!(text.contains(r#""deleted":true"#));
    });
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with("DELETE /v1/files/file-abc"));
}

#[test]
fn a_retrieve_call_gets_the_file_and_hands_out_the_object() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], CREATED),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let text = session
            .retrieve(py, &retrieve_payload(&socket), Vec::new())
            .unwrap();
        assert!(text.contains(r#""id":"file-abc""#));
        assert!(text.contains(r#""filename":"x.jsonl""#));
    });
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with("GET /v1/files/file-abc"));
}

#[test]
fn a_list_call_gets_the_page_and_hands_out_its_rows() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], PAGE),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let text = session
            .list(py, &list_payload(&socket), Vec::new())
            .unwrap();
        assert!(text.contains(r#""object":"list""#));
        assert!(text.contains(r#""has_more":false"#));
        assert!(text.contains(r#""id":"file-abc""#));
    });
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with("GET /v1/files?purpose=batch"));
}

#[test]
fn a_missing_file_is_the_family_exception_naming_not_found() {
    Python::initialize();
    let serving = serve(
        reply(
            "404 Not Found",
            &[("content-type", "application/json")],
            r#"{"error":{"message":"No such file","type":"invalid_request_error","code":"file_not_found"}}"#,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let error = session
            .retrieve(py, &retrieve_payload(&socket), Vec::new())
            .err()
            .unwrap();
        assert!(error.is_instance_of::<NativeFileCallError>(py));
        let text = error.to_string();
        assert!(text.contains(r#""type":"NotFoundError""#));
        assert!(text.contains(r#""status_code":404"#));
        assert!(text.contains(r#""code":"file_not_found""#));
    });
    serving.finish().unwrap();
}

#[test]
fn a_deadline_that_expires_is_the_connection_failure() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], CREATED),
        Duration::from_millis(200),
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let payload = format!(
            r#"{{"file_id":"file-abc","custom_llm_provider":"openai","params":{{"api_base":"http://{socket}/v1","api_key":"sk-test","timeout":0.01}}}}"#
        );
        let error = session.retrieve(py, &payload, Vec::new()).err().unwrap();
        assert!(error.is_instance_of::<NativeFileCallError>(py));
        assert!(error.to_string().contains(r#""type":"APIConnectionError""#));
        assert!(error.to_string().contains(r#""status_code":500"#));
    });
    serving.finish().unwrap();
}

#[test]
fn a_rejected_create_provider_is_the_family_exception_naming_bad_request() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let payload =
            r#"{"file":{"$file":{"blob":0}},"purpose":"batch","custom_llm_provider":"cohere"}"#;
        let error = session
            .create(py, payload, vec![b"payload".to_vec()])
            .err()
            .unwrap();
        assert!(error.is_instance_of::<NativeFileCallError>(py));
        let text = error.to_string();
        assert!(text.contains(r#""type":"BadRequestError""#));
        assert!(text.contains("LiteLLM doesn't support cohere for 'create_file'"));
    });
}

#[test]
fn a_generic_create_provider_names_the_create_it_has_not_built() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let payload =
            r#"{"file":{"$file":{"blob":0}},"purpose":"batch","custom_llm_provider":"anthropic"}"#;
        let error = session
            .create(py, payload, vec![b"payload".to_vec()])
            .err()
            .unwrap();
        assert!(error.is_instance_of::<PyNotImplementedError>(py));
        assert!(error.to_string().contains("AnthropicFilesConfig"));
    });
}

#[test]
fn a_generic_retrieve_provider_names_the_retrieve_it_has_not_built() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let payload = r#"{"file_id":"file-abc","custom_llm_provider":"anthropic"}"#;
        let error = session.retrieve(py, payload, Vec::new()).err().unwrap();
        assert!(error.is_instance_of::<PyNotImplementedError>(py));
    });
}

#[test]
fn a_generic_list_provider_names_the_list_it_has_not_built() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileCallSession::new(ENVIRONMENT).unwrap();
        let payload = r#"{"custom_llm_provider":"anthropic"}"#;
        let error = session.list(py, payload, Vec::new()).err().unwrap();
        assert!(error.is_instance_of::<PyNotImplementedError>(py));
    });
}
