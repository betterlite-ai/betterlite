use serde_json::json;

use super::support::{self, answer, call, params, pipeline_with, response, target};
use crate::api::soniox::pipeline::error::PipelineError;
use crate::api::soniox::records::auth::MISSING_KEY;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

fn caller_file_answers() -> Vec<support::Reply> {
    vec![
        answer(200, r#"{"id":"tr_1"}"#),
        answer(200, r#"{"status":"completed","audio_duration_ms":1500}"#),
        answer(200, r#"{"text":"hello"}"#),
        answer(200, "{}"),
    ]
}

#[tokio::test]
async fn a_sync_call_keeps_a_supplied_sync_client() {
    let supplied_effects = support::Effects::default();
    let provider_effects = support::Effects::default();
    let book = support::LogBook::default();
    let supplied = support::Recorder::new(caller_file_answers(), &supplied_effects);
    let provider = support::Recorder::new(Vec::new(), &provider_effects);
    let pipeline = pipeline_with(&provider, &supplied);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&supplied_effects, &book);
    let supplied_client = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let outcome = pipeline
        .transcribe(
            &call,
            target(None, &mut response, &mut logger),
            Some(supplied_client),
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(supplied.calls().len(), 4);
    assert!(provider.calls().is_empty());
    assert!(provider_effects.kinds().is_empty());
    assert_eq!(response.text.as_deref(), Some("hello"));
}

#[tokio::test]
async fn a_sync_call_replaces_a_supplied_async_client() {
    let supplied_effects = support::Effects::default();
    let provider_effects = support::Effects::default();
    let book = support::LogBook::default();
    let supplied = support::Recorder::new(caller_file_answers(), &supplied_effects);
    let provider = support::Recorder::new(caller_file_answers(), &provider_effects);
    let pipeline = pipeline_with(&provider, &provider);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&provider_effects, &book);
    let supplied_client = SuppliedClient {
        kind: ClientKind::Asynchronous,
        transport: &supplied,
    };
    let outcome = pipeline
        .transcribe(
            &call,
            target(None, &mut response, &mut logger),
            Some(supplied_client),
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(supplied.calls().is_empty());
    assert!(supplied_effects.kinds().is_empty());
    assert_eq!(provider.calls().len(), 4);
    assert_eq!(response.text.as_deref(), Some("hello"));
}

#[tokio::test]
async fn an_async_call_keeps_a_supplied_async_client() {
    let supplied_effects = support::Effects::default();
    let provider_effects = support::Effects::default();
    let book = support::LogBook::default();
    let supplied = support::Recorder::new(caller_file_answers(), &supplied_effects);
    let provider = support::Recorder::new(Vec::new(), &provider_effects);
    let pipeline = pipeline_with(&provider, &supplied);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let call = call(&case_params, true);
    let mut response = response();
    let mut logger = support::Logger::new(&supplied_effects, &book);
    let supplied_client = SuppliedClient {
        kind: ClientKind::Asynchronous,
        transport: &supplied,
    };
    let outcome = pipeline
        .transcribe(
            &call,
            target(None, &mut response, &mut logger),
            Some(supplied_client),
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(supplied.calls().len(), 4);
    assert!(provider.calls().is_empty());
    assert!(provider_effects.kinds().is_empty());
}

#[tokio::test]
async fn an_async_call_replaces_a_supplied_sync_client() {
    let supplied_effects = support::Effects::default();
    let provider_effects = support::Effects::default();
    let book = support::LogBook::default();
    let supplied = support::Recorder::new(caller_file_answers(), &supplied_effects);
    let provider = support::Recorder::new(caller_file_answers(), &provider_effects);
    let pipeline = pipeline_with(&provider, &provider);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!("caller-file")),
    ]);
    let call = call(&case_params, true);
    let mut response = response();
    let mut logger = support::Logger::new(&provider_effects, &book);
    let supplied_client = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let outcome = pipeline
        .transcribe(
            &call,
            target(None, &mut response, &mut logger),
            Some(supplied_client),
        )
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(supplied.calls().is_empty());
    assert!(supplied_effects.kinds().is_empty());
    assert_eq!(provider.calls().len(), 4);
    assert_eq!(response.text.as_deref(), Some("hello"));
}

#[tokio::test]
async fn a_missing_api_key_returns_the_prepare_soniox_401() {
    let effects = support::Effects::default();
    let book = support::LogBook::default();
    let recorder = support::Recorder::new(Vec::new(), &effects);
    let pipeline = pipeline_with(&recorder, &recorder);
    let case_params = params(&[("file_id", json!("caller-file"))]);
    let mut call = call(&case_params, false);
    call.api_key = None;
    let mut response = response();
    let mut logger = support::Logger::new(&effects, &book);
    let outcome = pipeline
        .transcribe::<support::Recorder<'_>>(&call, target(None, &mut response, &mut logger), None)
        .await;

    let error = outcome.expect_err("a call without an API key cannot be authorized");
    let exception = error
        .provider()
        .expect("the auth failure is the Soniox 401");
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.message, MISSING_KEY);
    assert!(matches!(error, PipelineError::Prepare(_)));
    assert!(recorder.calls().is_empty());
    assert!(effects.kinds().is_empty());
    assert!(response.hidden_params.is_empty());
}

#[tokio::test]
async fn a_prepare_failure_runs_no_request_and_no_cleanup() {
    let supplied_effects = support::Effects::default();
    let provider_effects = support::Effects::default();
    let book = support::LogBook::default();
    let supplied = support::Recorder::new(caller_file_answers(), &supplied_effects);
    let provider = support::Recorder::new(caller_file_answers(), &provider_effects);
    let pipeline = pipeline_with(&provider, &provider);
    let case_params = params(&[
        ("soniox_max_polling_attempts", json!(1)),
        ("file_id", json!(5)),
    ]);
    let call = call(&case_params, false);
    let mut response = response();
    let mut logger = support::Logger::new(&provider_effects, &book);
    let supplied_client = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let outcome = pipeline
        .transcribe(
            &call,
            target(None, &mut response, &mut logger),
            Some(supplied_client),
        )
        .await;

    assert!(matches!(
        outcome.expect_err("a non-string file_id is a prepare error"),
        PipelineError::Prepare(_)
    ));
    assert!(supplied.calls().is_empty());
    assert!(provider.calls().is_empty());
    assert!(supplied_effects.kinds().is_empty());
    assert!(provider_effects.kinds().is_empty());
    assert!(response.hidden_params.is_empty());
}
