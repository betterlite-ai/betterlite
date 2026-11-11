#![cfg(test)]

use secrecy::ExposeSecret;

use super::cases::{Run, auth_params, call, run_with_source};
use super::fixture::{Reply, Source};
use crate::image_call::error::ImageCallError;
use crate::image_call::transport::ClientKind;
use crate::image_models::params::{Field, Params};

const FIRST_ACCESS: &str = "AKIAFIRSTEXAMPLE00000";
const FIRST_SECRET: &str = "firstSecretKey0000000000000000000000000000";
const SECOND_ACCESS: &str = "AKIASECONDEXAMPLE0000";
const SECOND_SECRET: &str = "secondSecretKey000000000000000000000000000";

fn params_for(access: &str, secret_key: &str, guardrail: &str) -> Params {
    let mut params = auth_params();
    params.insert("aws_access_key_id".to_owned(), Field::from(access));
    params.insert("aws_secret_access_key".to_owned(), Field::from(secret_key));
    params.insert("guardrailIdentifier".to_owned(), Field::from(guardrail));
    params
}

fn secrets_in(text: &str, secrets: &[&str]) -> Vec<String> {
    secrets
        .iter()
        .filter(|secret| text.contains(**secret))
        .map(|secret| (*secret).to_owned())
        .collect()
}

fn identity_run(access: &str, secret_key: &str, guardrail: &str, source: &Source) -> Run {
    run_with_source(
        call(
            "amazon.titan-image-generator-v2:0",
            "a cat",
            params_for(access, secret_key, guardrail),
        ),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
        source,
    )
}

#[test]
fn image_call_credential_identity_isolation() {
    let first_source = Source::with(FIRST_ACCESS, FIRST_SECRET);
    let second_source = Source::with(SECOND_ACCESS, SECOND_SECRET);
    let first = identity_run(FIRST_ACCESS, FIRST_SECRET, "first-guardrail", &first_source);
    let second = identity_run(
        SECOND_ACCESS,
        SECOND_SECRET,
        "second-guardrail",
        &second_source,
    );
    let first_authorization = &first.headers()["Authorization"];
    let second_authorization = &second.headers()["Authorization"];
    assert!(first_authorization.contains(FIRST_ACCESS));
    assert!(second_authorization.contains(SECOND_ACCESS));
    assert_ne!(first_authorization, second_authorization);
    assert_eq!(
        first.headers()["x-amz-bedrock-guardrail-identifier"],
        "first-guardrail"
    );
    assert_eq!(
        second.headers()["x-amz-bedrock-guardrail-identifier"],
        "second-guardrail"
    );
}

#[test]
fn image_call_resolver_identities_do_not_cross() {
    let first_source = Source::with(FIRST_ACCESS, FIRST_SECRET);
    let second_source = Source::with(SECOND_ACCESS, SECOND_SECRET);
    identity_run(FIRST_ACCESS, FIRST_SECRET, "gr", &first_source);
    identity_run(SECOND_ACCESS, SECOND_SECRET, "gr", &second_source);
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
fn image_call_secret_safe_failure() {
    let access = "AKIALEAKEXAMPLE00000";
    let secret_key = "leakSecretKey0000000000000000000000000000";
    let run = identity_run(access, secret_key, "gr", &Source::with(access, secret_key));
    let request_debug = format!("{:?}", run.posted());
    assert!(request_debug.contains("header_names"));
    assert_eq!(
        secrets_in(&request_debug, &[access, secret_key]),
        Vec::<String>::new()
    );
    let state_text = serde_json::to_string(&run.state.model_call_details).unwrap();
    assert_eq!(
        secrets_in(&state_text, &[access, secret_key]),
        Vec::<String>::new()
    );
}

#[test]
fn image_call_failure_paths_hide_credentials() {
    let access = "AKIALEAKEXAMPLE00000";
    let secret_key = "leakSecretKey0000000000000000000000000000";
    let failing = run_with_source(
        call(
            "amazon.titan-image-generator-v2:0",
            "a cat",
            params_for(access, secret_key, "gr"),
        ),
        Reply::status(403, &[("x-amzn-errortype", "AccessDenied")], "denied"),
        Some(ClientKind::Sync),
        &Source::with(access, secret_key),
    );
    let error = failing.result.as_ref().unwrap_err();
    assert!(matches!(
        error,
        ImageCallError::Provider { status: 403, .. }
    ));
    let error_debug = format!("{error:?}");
    let error_display = error.to_string();
    assert_eq!(
        secrets_in(&error_debug, &[access, secret_key]),
        Vec::<String>::new()
    );
    assert_eq!(
        secrets_in(&error_display, &[access, secret_key]),
        Vec::<String>::new()
    );
}
