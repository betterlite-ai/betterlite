#![cfg(test)]

use std::io;
use std::string::FromUtf8Error;

use futures_util::stream;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::{
    CodeExecutionResult, ContainerHandle, ExecutionError, HiddenParams, SANDBOX_MAX_OUTPUT_BYTES,
    SandboxOutputError, read_capped_lines,
};

fn raw(text: &str) -> Box<RawValue> {
    serde_json::from_str(text).unwrap()
}

#[test]
fn container_handle_serializes_identity_and_keeps_hidden_params_private() {
    let handle = ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "e2b".to_owned(),
        domain: None,
        hidden_params: HiddenParams {
            envd_access_token: Some(SecretString::from("envd-token")),
            traffic_access_token: None,
            api_key: Some(SecretString::from("e2b-key")),
            api_base: Some("https://api.e2b.app".to_owned()),
        },
    };
    let json = serde_json::to_string(&handle).unwrap();
    assert_eq!(json, r#"{"id":"sbx_1","provider":"e2b","domain":null}"#);
    assert!(!json.contains("envd-token"));
    assert!(!json.contains("e2b-key"));
}

#[test]
fn code_execution_result_defaults_to_the_code_execution_object() {
    let result = CodeExecutionResult::default();
    assert_eq!(result.object, "code_execution");
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"stdout":"","stderr":"","results":[],"error":null,"execution_count":null,"object":"code_execution"}"#
    );
}

#[test]
fn code_execution_result_writes_error_details_and_count_as_json() {
    let result = CodeExecutionResult {
        stdout: "out".to_owned(),
        stderr: "err".to_owned(),
        results: Vec::new(),
        error: Some(ExecutionError {
            name: Some(raw(r#""ValueError""#)),
            value: Some(raw(r#""bad input""#)),
            traceback: None,
        }),
        execution_count: Some(2),
        object: "code_execution".to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"stdout":"out","stderr":"err","results":[],"error":{"name":"ValueError","value":"bad input","traceback":null},"execution_count":2,"object":"code_execution"}"#
    );
}

#[tokio::test]
async fn read_capped_lines_splits_chunks_and_keeps_encounter_order() {
    let chunks = stream::iter(vec![
        Ok::<Vec<u8>, io::Error>(b"al".to_vec()),
        Ok(b"pha\r\nbeta\ngamma".to_vec()),
    ]);
    assert_eq!(
        read_capped_lines(chunks).await.unwrap(),
        vec!["alpha", "beta", "gamma"]
    );
}

#[tokio::test]
async fn read_capped_lines_keeps_empty_lines_and_drops_the_final_separator() {
    let chunks = stream::iter(vec![Ok::<Vec<u8>, io::Error>(b"a\n\nb\n".to_vec())]);
    assert_eq!(read_capped_lines(chunks).await.unwrap(), vec!["a", "", "b"]);
}

#[tokio::test]
async fn read_capped_lines_accepts_exactly_the_limit() {
    let line = "a".repeat(SANDBOX_MAX_OUTPUT_BYTES);
    let chunks = stream::iter(vec![Ok::<Vec<u8>, io::Error>(line.into_bytes())]);
    assert_eq!(
        read_capped_lines(chunks).await.unwrap(),
        vec!["a".repeat(SANDBOX_MAX_OUTPUT_BYTES)]
    );
}

#[tokio::test]
async fn read_capped_lines_stops_at_the_ten_mib_boundary_and_names_it() {
    let mut data = "a".repeat(SANDBOX_MAX_OUTPUT_BYTES).into_bytes();
    data.push(b'\n');
    data.extend_from_slice(b"b\n");
    let chunks = stream::iter(vec![
        Ok::<Vec<u8>, io::Error>(data),
        Err(io::Error::other("a chunk the reader must never request")),
    ]);
    let error = read_capped_lines(chunks).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxOutputError::Exceeded { limit } if limit == SANDBOX_MAX_OUTPUT_BYTES
    ));
    assert!(error.to_string().contains("10485760"));
}

#[tokio::test]
async fn read_capped_lines_rejects_an_unterminated_line_past_the_limit() {
    let chunks = stream::iter(vec![Ok::<Vec<u8>, io::Error>(vec![
        b'a';
        SANDBOX_MAX_OUTPUT_BYTES
            + 1
    ])]);
    assert!(matches!(
        read_capped_lines(chunks).await.unwrap_err(),
        SandboxOutputError::Exceeded { limit } if limit == SANDBOX_MAX_OUTPUT_BYTES
    ));
}

#[tokio::test]
async fn read_capped_lines_reports_output_that_is_not_utf8() {
    let chunks = stream::iter(vec![Ok::<Vec<u8>, io::Error>(vec![0xff, b'\n'])]);
    let error = read_capped_lines(chunks).await.unwrap_err();
    assert!(matches!(
        error,
        SandboxOutputError::Utf8 {
            source: FromUtf8Error { .. }
        }
    ));
}

#[tokio::test]
async fn read_capped_lines_keeps_the_stream_failure_as_its_source() {
    let chunks = stream::iter(vec![Err::<Vec<u8>, io::Error>(io::Error::other("gone"))]);
    let error = read_capped_lines(chunks).await.unwrap_err();
    match error {
        SandboxOutputError::Read { source } => assert_eq!(source.to_string(), "gone"),
        SandboxOutputError::Exceeded { .. } | SandboxOutputError::Utf8 { .. } => {
            panic!("the stream failure must surface as a read error")
        }
    }
}
