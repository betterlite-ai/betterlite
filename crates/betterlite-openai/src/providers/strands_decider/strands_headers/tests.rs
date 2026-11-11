#![cfg(test)]

use std::fs::read_to_string;
use std::sync::Arc;

use betterlite_models::decisions::Extra;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::arn::agentcore_runtime;
use super::headers::{SESSION_HEADER, runtime_headers};
use crate::openai_base_config::Headers;

const ARN: &str = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime";

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("a reopened log")))
        .finish();
    let result = with_default(subscriber, run);
    (
        result,
        read_to_string(log.path()).expect("the logged warnings"),
    )
}

#[test]
fn strands_headers_drop_the_host_and_both_reserved_prefixes_case_insensitively() {
    let runtime = agentcore_runtime(ARN).expect("a runtime ARN");
    let caller = headers(&[
        ("Host", "evil.example"),
        ("host", "also.evil.example"),
        ("X-Amzn-Bedrock-AgentCore-Runtime-User-Id", "someone-else"),
        ("X-Amz-Security-Token", "caller-token"),
        ("x-amz-target", "forged"),
        ("X-Request-Source", "batch"),
        ("x-amazon", "kept"),
        ("x-amzn-bedrock-agentcore", "kept"),
    ]);
    let session = runtime_headers(&caller, &runtime, &Extra::new());
    assert_eq!(
        session.dropped,
        vec![
            "Host".to_owned(),
            "X-Amz-Security-Token".to_owned(),
            "X-Amzn-Bedrock-AgentCore-Runtime-User-Id".to_owned(),
            "host".to_owned(),
            "x-amz-target".to_owned(),
        ]
    );
    let kept: Vec<&str> = session.headers.keys().map(String::as_str).collect();
    assert_eq!(
        kept,
        vec![
            SESSION_HEADER,
            "X-Request-Source",
            "x-amazon",
            "x-amzn-bedrock-agentcore",
        ]
    );
    assert_eq!(
        session.headers.get(SESSION_HEADER).map(String::as_str),
        Some("litellm-decider-92976cdcb2dfb49b5759fc567c6cc5241c0c5d52482fd1e0d7db5116ae4c5c20")
    );
}

#[test]
fn strands_headers_warn_once_with_the_dropped_names() {
    let runtime = agentcore_runtime(ARN).expect("a runtime ARN");
    let caller = headers(&[("Host", "evil.example"), ("x-amz-target", "forged")]);
    let (_, warnings) = capture_warnings(|| runtime_headers(&caller, &runtime, &Extra::new()));
    assert!(
        warnings.contains("not forwarding reserved AgentCore headers"),
        "{warnings}"
    );
    assert!(warnings.contains("x-amz-target"), "{warnings}");
    assert_eq!(warnings.matches("not forwarding reserved").count(), 1);
}

#[test]
fn strands_headers_keep_names_that_only_resemble_a_reserved_prefix() {
    let runtime = agentcore_runtime(ARN).expect("a runtime ARN");
    let caller = headers(&[
        ("x-amazons3", "kept"),
        ("x-amzn-bedrock-agentcore", "kept"),
        ("x-amz", "kept"),
    ]);
    let session = runtime_headers(&caller, &runtime, &Extra::new());
    assert!(session.dropped.is_empty());
    assert_eq!(session.headers.len(), 4);
    assert_eq!(
        session.headers.get("x-amazons3").map(String::as_str),
        Some("kept")
    );
}

#[test]
fn strands_headers_do_not_warn_when_no_header_is_reserved() {
    let runtime = agentcore_runtime(ARN).expect("a runtime ARN");
    let caller = headers(&[("X-Request-Source", "batch")]);
    let (_, warnings) = capture_warnings(|| runtime_headers(&caller, &runtime, &Extra::new()));
    assert!(!warnings.contains("not forwarding reserved"), "{warnings}");
}
