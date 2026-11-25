#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::super::ENVIRONMENT_VARIABLES;
use super::super::environment::EnvironmentError;
use super::super::error::RotationError;
use super::support::{
    Failure, FakeStoreError, NEW_KEY, OLD_KEY, config_row, credential_row, credential_writes,
    fixture, object, open, saved_environment, sealed, sealed_by, text,
};

// _decrypt_and_set_db_env_variables exports the plaintext and _encrypt_env_variables seals it again, so the row keeps no plaintext.
#[tokio::test]
async fn environment_values_are_decrypted_applied_and_resealed() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        object(vec![
            ("OPENAI_API_KEY", text(&sealed("sk-live"))),
            ("LITELLM_ENABLE_MCP_STDIO", text(&sealed("true"))),
        ]),
    )];
    fixture.rotation().run(Some("user-1")).await.unwrap();
    let written =
        saved_environment(&fixture.store.writes()).expect("the environment row is written");
    let JsonData::Object(stored) = written else {
        panic!("the stored mapping is an object");
    };
    let JsonData::String(api_key) = &stored["OPENAI_API_KEY"] else {
        panic!("the stored api key is text");
    };
    assert_eq!(open(api_key, NEW_KEY).as_deref(), Some("sk-live"));
    assert_eq!(open(api_key, OLD_KEY), None);
    assert!(!stored.contains_key("LITELLM_ENABLE_MCP_STDIO"));
    let snapshot = fixture.runtime.snapshot();
    assert_eq!(
        snapshot.values.get("OPENAI_API_KEY").map(String::as_str),
        Some("sk-live")
    );
    assert!(!snapshot.values.contains_key("LITELLM_ENABLE_MCP_STDIO"));
    // the read path's once-only warning ran: the stdio flag reached the refusal branch
    assert!(!fixture.warnings.first_stdio_flag());
}

// ConfigRepository.set_param stores the mapping as JSON text; LiteLLM's dict[str, str](str) raises on it and betterlite decodes it.
#[tokio::test]
async fn a_config_row_stored_as_json_text_is_decoded() {
    let mut fixture = fixture();
    let mapping = format!(r#"{{"OPENAI_API_KEY":"{}"}}"#, sealed("sk-live"));
    fixture.store.config = vec![config_row(ENVIRONMENT_VARIABLES, JsonData::String(mapping))];
    fixture.rotation().run(None).await.unwrap();
    let written =
        saved_environment(&fixture.store.writes()).expect("the environment row is written");
    let JsonData::Object(stored) = written else {
        panic!("the stored mapping is an object");
    };
    let JsonData::String(api_key) = &stored["OPENAI_API_KEY"] else {
        panic!("the stored api key is text");
    };
    assert_eq!(open(api_key, NEW_KEY).as_deref(), Some("sk-live"));
}

// decrypt_value_helper returns nothing for a value it cannot open, so that key is dropped and the rest rotate.
#[tokio::test]
async fn an_unreadable_environment_value_is_dropped_and_the_rest_rotate() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        object(vec![
            ("GOOD", text(&sealed("sk-good"))),
            ("BAD", text(&sealed_by("another-master-key", "sk-bad"))),
        ]),
    )];
    fixture.rotation().run(None).await.unwrap();
    let written =
        saved_environment(&fixture.store.writes()).expect("the environment row is written");
    let JsonData::Object(stored) = written else {
        panic!("the stored mapping is an object");
    };
    assert!(stored.contains_key("GOOD"));
    assert!(!stored.contains_key("BAD"));
    let snapshot = fixture.runtime.snapshot();
    assert_eq!(
        snapshot.values.get("GOOD").map(String::as_str),
        Some("sk-good")
    );
    assert!(!snapshot.values.contains_key("BAD"));
}

// Nothing decrypted means an empty encrypted mapping, which LiteLLM's `if encrypted_env_vars:` skips.
#[tokio::test]
async fn a_config_row_with_no_readable_values_is_not_written() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        object(vec![(
            "BAD",
            text(&sealed_by("another-master-key", "sk-bad")),
        )]),
    )];
    fixture.rotation().run(None).await.unwrap();
    assert!(saved_environment(&fixture.store.writes()).is_none());
    assert!(fixture.runtime.snapshot().values.is_empty());
}

#[tokio::test]
async fn an_empty_environment_mapping_is_not_written() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(ENVIRONMENT_VARIABLES, object(Vec::new()))];
    fixture.rotation().run(None).await.unwrap();
    assert!(saved_environment(&fixture.store.writes()).is_none());
}

#[tokio::test]
async fn a_null_environment_row_is_skipped() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(ENVIRONMENT_VARIABLES, JsonData::Null)];
    fixture.rotation().run(None).await.unwrap();
    assert!(saved_environment(&fixture.store.writes()).is_none());
}

// LiteLLM's loop reassigns on every matching row, so the last environment_variables row wins.
#[tokio::test]
async fn the_last_environment_row_wins() {
    let mut fixture = fixture();
    fixture.store.config = vec![
        config_row(
            ENVIRONMENT_VARIABLES,
            object(vec![("FIRST_ONLY", text(&sealed("one")))]),
        ),
        config_row(
            ENVIRONMENT_VARIABLES,
            object(vec![("SECOND_ONLY", text(&sealed("two")))]),
        ),
    ];
    fixture.rotation().run(None).await.unwrap();
    let written =
        saved_environment(&fixture.store.writes()).expect("the environment row is written");
    let JsonData::Object(stored) = written else {
        panic!("the stored mapping is an object");
    };
    assert!(stored.contains_key("SECOND_ONLY"));
    assert!(!stored.contains_key("FIRST_ONLY"));
    let snapshot = fixture.runtime.snapshot();
    assert_eq!(
        snapshot.values.get("SECOND_ONLY").map(String::as_str),
        Some("two")
    );
    assert!(!snapshot.values.contains_key("FIRST_ONLY"));
}

#[tokio::test]
async fn a_config_scan_failure_skips_only_the_environment_stage() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::ConfigScan];
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-1")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    assert!(saved_environment(&fixture.store.writes()).is_none());
    assert!(fixture.runtime.snapshot().values.is_empty());
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 1);
}

#[tokio::test]
async fn an_environment_row_that_is_not_an_object_is_an_error() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        JsonData::Array(Vec::new()),
    )];
    let error = fixture.rotation().run(None).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Environment(EnvironmentError::NotAnObject)
    ));
}

#[tokio::test]
async fn an_environment_row_that_does_not_decode_is_an_error() {
    let mut fixture = fixture();
    fixture.store.config = vec![config_row(
        ENVIRONMENT_VARIABLES,
        JsonData::String("not json".to_owned()),
    )];
    let error = fixture.rotation().run(None).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Environment(EnvironmentError::Decode(_))
    ));
}

// The runtime is changed before the row is written, so a failed write leaves the deployment using the new values.
#[tokio::test]
async fn the_runtime_keeps_the_values_when_the_config_write_fails() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::SaveEnvironment];
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
    assert_eq!(
        fixture
            .runtime
            .snapshot()
            .values
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("sk-live")
    );
    assert!(saved_environment(&fixture.store.writes()).is_none());
}
