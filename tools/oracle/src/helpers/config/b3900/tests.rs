#![cfg(test)]

use serde_json::json;

use super::Op;
use crate::helpers::Helper as _;

#[test]
fn credentials_prefer_the_arguments() {
    let answer = Op::decode_and_run(&json!({
        "function": "credentials",
        "steps": [{
            "public_key": "arg-public",
            "secret": "arg-secret",
            "secret_key": null,
            "host": null,
            "allow_env_credentials": true,
            "environment": {"LANGFUSE_PUBLIC_KEY": "env-public", "LANGFUSE_SECRET_KEY": "env-secret"},
        }],
    }))
    .expect("the credentials call runs");

    assert_eq!(answer[0]["ok"], json!(true));
    assert_eq!(answer[0]["value"]["public_key"], json!("arg-public"));
    assert_eq!(answer[0]["value"]["secret_key"], json!("arg-secret"));
    assert_eq!(
        answer[0]["value"]["host"],
        json!("https://cloud.langfuse.com")
    );
}

#[test]
fn a_host_without_a_scheme_gets_http() {
    let answer = Op::decode_and_run(&json!({
        "function": "host",
        "steps": [{"host": "langfuse.internal:3000", "environment": {}}],
    }))
    .expect("the host call runs");

    assert_eq!(answer[0]["value"], json!("http://langfuse.internal:3000"));
}

#[test]
fn a_read_reports_the_warning_beside_the_fallback() {
    let answer = Op::decode_and_run(&json!({
        "function": "read",
        "reading": "max_retries",
        "steps": [{"environment": {"LANGFUSE_MAX_RETRIES": "1001"}}],
    }))
    .expect("the read call runs");

    assert_eq!(answer[0]["ok"], json!(true));
    assert_eq!(answer[0]["value"], json!(1000_u32));
    assert_eq!(
        answer[0]["warnings"],
        json!(["LANGFUSE_MAX_RETRIES=1001 is above the ceiling; retrying 1000 times"])
    );
    assert_eq!(answer[0]["warning_details"], json!([""]));
}

#[test]
fn an_invalid_explicit_environment_is_reported_as_failed() {
    let answer = Op::decode_and_run(&json!({
        "function": "environment",
        "steps": [{"explicit": "Bad Env", "environment": {}}],
    }))
    .expect("the environment call runs");

    assert_eq!(answer[0]["ok"], json!(false));
    assert_eq!(answer[0]["value"], json!(null));
}

#[test]
fn a_mock_line_is_reported_as_information() {
    let answer = Op::decode_and_run(&json!({
        "function": "read",
        "reading": "debug_and_mock",
        "steps": [{"environment": {"LANGFUSE_MOCK": "true"}}],
    }))
    .expect("the read call runs");

    assert_eq!(answer[0]["value"], json!({"debug": false, "mock": true}));
    assert_eq!(
        answer[0]["infos"],
        json!(["LANGFUSE Mock Mode: ENABLED - API calls will be mocked"])
    );
}
