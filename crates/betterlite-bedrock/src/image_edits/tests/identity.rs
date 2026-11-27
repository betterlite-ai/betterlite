#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::Redactor;
use secrecy::ExposeSecret;
use serde_json::Number;

use super::cases::{Answer, Run, auth_params, call, run_with_source};
use super::fixture::{Reply, Source};
use crate::image_call::transport::ClientKind;
use crate::image_edits::error::ImageEditError;
use crate::image_edits::plan::EditImage;

const FIRST_ACCESS: &str = "AKIAFIRSTEXAMPLE00000";
const FIRST_SECRET: &str = "firstSecretKey0000000000000000000000000000";
const SECOND_ACCESS: &str = "AKIASECONDEXAMPLE0000";
const SECOND_SECRET: &str = "secondSecretKey000000000000000000000000000";
const EDIT_MODEL: &str = "stability.stable-image-inpaint-v1:0";

fn params_for(access: &str, secret_key: &str, seed: i64) -> ImageParams {
    let mut params = auth_params();
    params.insert(
        "aws_access_key_id".to_owned(),
        Json::Text(access.to_owned()),
    );
    params.insert(
        "aws_secret_access_key".to_owned(),
        Json::Text(secret_key.to_owned()),
    );
    params.insert("seed".to_owned(), Json::Number(Number::from(seed)));
    params
}

fn secrets_in(text: &str, secrets: &[&str]) -> Vec<String> {
    secrets
        .iter()
        .filter(|secret| text.contains(**secret))
        .map(|secret| (*secret).to_owned())
        .collect()
}

fn identity_run(
    access: &str,
    secret_key: &str,
    seed: i64,
    source: &Source,
    redactor: &Redactor,
) -> Run {
    let images = vec![EditImage::Text("aW1n".to_owned())];
    let mut edit_call = call(
        EDIT_MODEL,
        Some("edit it"),
        params_for(access, secret_key, seed),
    );
    edit_call.images = &images;
    run_with_source(
        edit_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: Some(ClientKind::Sync),
        },
        source,
        redactor,
    )
}

#[test]
fn image_edit_call_credential_identity_isolation() {
    let redactor = &super::fixture::redactor();
    let first_source = Source::with(FIRST_ACCESS, FIRST_SECRET);
    let second_source = Source::with(SECOND_ACCESS, SECOND_SECRET);
    let first = identity_run(FIRST_ACCESS, FIRST_SECRET, 1, &first_source, redactor);
    let second = identity_run(SECOND_ACCESS, SECOND_SECRET, 2, &second_source, redactor);
    assert_distinct_authorization(&first, &second);
    assert_own_seed(&first, 1_i64);
    assert_own_seed(&second, 2_i64);
    assert_resolved_identity(&first_source, FIRST_ACCESS);
    assert_resolved_identity(&second_source, SECOND_ACCESS);
}

fn assert_distinct_authorization(first: &Run, second: &Run) {
    let first_headers = first.headers();
    let second_headers = second.headers();
    assert!(first_headers["Authorization"].contains(FIRST_ACCESS));
    assert!(second_headers["Authorization"].contains(SECOND_ACCESS));
    assert_ne!(
        first_headers["Authorization"],
        second_headers["Authorization"]
    );
}

fn assert_own_seed(run: &Run, seed: i64) {
    assert_eq!(
        run.body().get("seed"),
        Some(&Json::Number(Number::from(seed)))
    );
}

fn assert_resolved_identity(source: &Source, access: &str) {
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "each call resolves its own identity");
    assert_eq!(
        calls[0]
            .0
            .access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(access)
    );
    assert_eq!(calls[0].1, "us-east-1");
}

#[test]
fn image_edit_call_secret_safe_failure() {
    let redactor = &super::fixture::redactor();
    let access = "AKIALEAKEXAMPLE00000";
    let secret_key = "leakSecretKey0000000000000000000000000000";
    let run = identity_run(
        access,
        secret_key,
        1,
        &Source::with(access, secret_key),
        redactor,
    );
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
fn image_edit_failure_paths_hide_credentials() {
    let access = "AKIALEAKEXAMPLE00000";
    let secret_key = "leakSecretKey0000000000000000000000000000";
    let failing = run_with_source(
        call(
            EDIT_MODEL,
            Some("edit it"),
            params_for(access, secret_key, 1),
        ),
        "{}",
        Answer {
            reply: Reply::status(403, &[("x-amzn-errortype", "AccessDenied")], "denied"),
            supplied: Some(ClientKind::Sync),
        },
        &Source::with(access, secret_key),
        &super::fixture::redactor(),
    );
    let error = failing.error();
    assert!(matches!(
        error,
        ImageEditError::Provider { status: 403, .. }
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
