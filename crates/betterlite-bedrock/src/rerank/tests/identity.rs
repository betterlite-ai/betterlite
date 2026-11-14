#![cfg(test)]

use betterlite_models::decisions::Json;
use secrecy::ExposeSecret;

use super::cases::{Run, call, params, request, run_with_source, text};
use super::fixture::{Reply, Source};
use crate::rerank::plan::ParamMap;
use crate::rerank::transport::ClientKind;

const VALID_REPLY: &str = r#"{"id":"resp-1","results":[{"index":0,"relevanceScore":0.5}]}"#;
const FIRST_ACCESS: &str = "AKIAFIRSTEXAMPLE00000";
const FIRST_SECRET: &str = "firstSecretKey0000000000000000000000000000";
const SECOND_ACCESS: &str = "AKIASECONDEXAMPLE0000";
const SECOND_SECRET: &str = "secondSecretKey000000000000000000000000000";

fn params_for(access: &str, secret_key: &str) -> ParamMap {
    params(&[
        ("aws_access_key_id", Json::Text(access.to_owned())),
        ("aws_secret_access_key", Json::Text(secret_key.to_owned())),
        ("aws_region_name", Json::Text("us-east-1".to_owned())),
    ])
}

fn identity_run(access: &str, secret_key: &str, source: &Source) -> Run {
    let documented = request(
        "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test",
        "q",
        vec![text("a")],
    );
    run_with_source(
        call(&documented, params_for(access, secret_key)),
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        source,
    )
}

fn secrets_in(text: &str, secrets: &[&str]) -> Vec<String> {
    secrets
        .iter()
        .filter(|secret| text.contains(**secret))
        .map(|secret| (*secret).to_owned())
        .collect()
}

#[test]
fn rerank_credential_identity_isolation() {
    let first_source = Source::with(FIRST_ACCESS, FIRST_SECRET);
    let second_source = Source::with(SECOND_ACCESS, SECOND_SECRET);
    let first = identity_run(FIRST_ACCESS, FIRST_SECRET, &first_source);
    let second = identity_run(SECOND_ACCESS, SECOND_SECRET, &second_source);
    let first_headers = first.headers();
    let second_headers = second.headers();
    let first_authorization = first_headers.get("Authorization").expect("signed");
    let second_authorization = second_headers.get("Authorization").expect("signed");
    assert!(first_authorization.contains(FIRST_ACCESS));
    assert!(second_authorization.contains(SECOND_ACCESS));
    assert_ne!(first_authorization, second_authorization);
    assert_eq!(
        first_headers.get("X-Amz-Date"),
        second_headers.get("X-Amz-Date"),
        "both calls sign at the same supplied instant"
    );
    let first_calls = first_source.calls.lock().unwrap();
    let second_calls = second_source.calls.lock().unwrap();
    assert_eq!(first_calls.len(), 1, "each call resolves its own identity");
    assert_eq!(second_calls.len(), 1);
    assert_eq!(
        first_calls[0]
            .0
            .access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(FIRST_ACCESS)
    );
    assert_eq!(
        second_calls[0]
            .0
            .access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SECOND_ACCESS)
    );
    assert_eq!(first_calls[0].1, "us-east-1");
    assert_eq!(second_calls[0].1, "us-east-1");
}

#[test]
fn rerank_secret_safe_failure() {
    let access = "AKIALEAKEXAMPLE00000";
    let secret_key = "leakSecretKey0000000000000000000000000000";
    let source = Source::with(access, secret_key);
    let documented = request(
        "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test",
        "q",
        vec![text("a")],
    );
    let run = run_with_source(
        call(&documented, params_for(access, secret_key)),
        Reply::status(403, &[("x-amzn-errortype", "AccessDenied")], "denied"),
        Some(ClientKind::Sync),
        &source,
    );
    let error = run.result.as_ref().unwrap_err();
    let request_debug = format!("{:?}", run.posted());
    assert!(request_debug.contains("header_names"));
    let shown = [
        format!("{error:?}"),
        error.to_string(),
        request_debug,
        serde_json::to_string(&run.state.model_call_details).expect("the details serialize"),
    ];
    for text in &shown {
        assert_eq!(
            secrets_in(text, &[access, secret_key]),
            Vec::<String>::new(),
            "a failure path hides the fixture credentials"
        );
    }
}
