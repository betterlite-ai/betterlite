#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::super::ENVIRONMENT_VARIABLES;
use super::super::error::RotationError;
use super::support::{
    Failure, FakeStoreError, NEW_KEY, OLD_KEY, SinkError, Write, config_row, credential_row,
    credential_writes, fixture, model_row, object, open, params_of, published, saved_environment,
    saved_models, sealed, text,
};

// The model stage seals every stored string with the explicit key and leaves the audit fields to the record prep.
#[tokio::test]
async fn model_values_are_reencrypted_with_the_new_key() {
    let mut fixture = fixture();
    fixture.store.models = vec![model_row(
        "m1",
        vec![
            ("model", text(&sealed("openai/gpt-4o"))),
            ("api_base", text(&sealed("https://example.test/v1"))),
        ],
    )];
    fixture.rotation().run(Some("user-1")).await.unwrap();
    let saved = saved_models(&fixture.store.writes());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].model_id, "m1");
    assert_eq!(saved[0].model_name, "gpt-4o");
    assert_eq!(saved[0].created_by, "user-1");
    assert_eq!(saved[0].updated_by, "user-1");
    let params = params_of(&saved[0]);
    let JsonData::String(model) = &params["model"] else {
        panic!("the model parameter is text");
    };
    assert_eq!(open(model, NEW_KEY).as_deref(), Some("openai/gpt-4o"));
    assert_eq!(open(model, OLD_KEY), None);
    let JsonData::String(api_base) = &params["api_base"] else {
        panic!("the api_base parameter is text");
    };
    assert_eq!(
        open(api_base, NEW_KEY).as_deref(),
        Some("https://example.test/v1")
    );
    assert_eq!(open(api_base, OLD_KEY), None);
}

// publish_config_change runs only after the model transaction committed, never before the write.
#[tokio::test]
async fn the_model_rows_are_saved_before_the_invalidation() {
    let mut fixture = fixture();
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    let writes = fixture.store.writes();
    assert_eq!(writes.len(), 2);
    assert!(matches!(writes[0], Write::Models(_)));
    assert!(matches!(writes[1], Write::Publish));
}

#[tokio::test]
async fn an_empty_model_scan_is_not_published() {
    let mut fixture = fixture();
    fixture.rotation().run(None).await.unwrap();
    assert!(fixture.store.writes().is_empty());
}

// LiteLLM catches the model scan and skips the stage; the credential stage still runs.
#[tokio::test]
async fn a_failed_model_scan_skips_only_the_model_stage() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::ModelScan];
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-1")))],
    )];
    fixture.rotation().run(Some("user-1")).await.unwrap();
    let writes = fixture.store.writes();
    assert!(saved_models(&writes).is_empty());
    assert!(!published(&writes));
    assert_eq!(credential_writes(&writes).len(), 1);
}

// decrypt_model_list_from_db logs an invalid deployment and drops it; the valid rows still rotate.
#[tokio::test]
async fn an_invalid_model_row_is_skipped_before_the_write() {
    let mut fixture = fixture();
    fixture.store.models = vec![
        model_row(
            "bad",
            vec![("api_base", text(&sealed("https://example.test/v1")))],
        ),
        model_row("good", vec![("model", text(&sealed("openai/gpt-4o")))]),
    ];
    fixture.rotation().run(None).await.unwrap();
    let saved = saved_models(&fixture.store.writes());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].model_id, "good");
}

// A failed model transaction propagates: no publish, no config write, no credential write.
#[tokio::test]
async fn a_failed_model_write_stops_the_rotation_before_publishing() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::SaveModels];
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-1")))],
    )];
    let error = fixture.rotation().run(Some("user-1")).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Store(FakeStoreError {
            failure: Failure::SaveModels
        })
    ));
    let writes = fixture.store.writes();
    assert!(!published(&writes));
    assert!(saved_environment(&writes).is_none());
    assert!(credential_writes(&writes).is_empty());
}

// The committed model rows stay when the invalidation fails.
#[tokio::test]
async fn a_failed_invalidation_keeps_the_saved_rows() {
    let mut fixture = fixture();
    fixture.sink.failed = true;
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-1")))],
    )];
    let error = fixture.rotation().run(None).await.unwrap_err();
    assert!(matches!(error, RotationError::Invalidation(SinkError)));
    let writes = fixture.store.writes();
    assert_eq!(saved_models(&writes).len(), 1);
    assert!(credential_writes(&writes).is_empty());
}

// LiteLLM enters the transaction and publishes even when every deployment was dropped before it.
#[tokio::test]
async fn an_all_invalid_model_scan_still_publishes() {
    let mut fixture = fixture();
    fixture.store.models = vec![model_row(
        "bad",
        vec![("api_base", text(&sealed("https://example.test/v1")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    let writes = fixture.store.writes();
    assert!(matches!(&writes[0], Write::Models(models) if models.is_empty()));
    assert!(published(&writes));
}

// A later stage's failure does not undo the models already committed and published.
#[tokio::test]
async fn a_failed_environment_write_keeps_the_model_rows() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::SaveEnvironment];
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        object(vec![("OPENAI_API_KEY", text(&sealed("sk-live")))]),
    )];
    let error = fixture.rotation().run(None).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Store(FakeStoreError {
            failure: Failure::SaveEnvironment
        })
    ));
    let writes = fixture.store.writes();
    assert_eq!(saved_models(&writes).len(), 1);
    assert!(published(&writes));
}
