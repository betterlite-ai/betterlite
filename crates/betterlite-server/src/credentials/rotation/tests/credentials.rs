#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    Failure, NEW_KEY, OLD_KEY, capture_logs, credential_row, credential_with, credential_writes,
    fixture, model_row, object, open, payload_values, saved_models, sealed, sealed_by, text,
};

fn stored_credential_json(info: Option<JsonData>) -> JsonData {
    let mut fields = vec![
        ("credential_name", text("c1")),
        (
            "credential_values",
            object(vec![("api_key", text(&sealed("sk-live")))]),
        ),
    ];
    if let Some(info_value) = info {
        fields.push(("credential_info", info_value));
    }
    object(fields)
}

// update_db_credential re-encrypts every value with the explicit key and writes the actor as updated_by.
#[tokio::test]
async fn credential_values_are_reencrypted_and_updated_by_is_the_actor() {
    let mut fixture = fixture();
    fixture.store.credentials = vec![credential_with(
        "c1",
        vec![("region", text("eu-west-1"))],
        vec![
            ("api_key", text(&sealed("sk-live"))),
            ("retries", JsonData::Number(Number::from(3_u32))),
        ],
    )];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    let writes = credential_writes(&fixture.store.writes());
    assert_eq!(writes.len(), 1);
    let (name, row, actor) = &writes[0];
    assert_eq!(name, "c1");
    assert_eq!(actor.as_deref(), Some("user-7"));
    let values = payload_values(row);
    let JsonData::String(api_key) = &values["api_key"] else {
        panic!("the stored api key is text");
    };
    assert_eq!(open(api_key, NEW_KEY).as_deref(), Some("sk-live"));
    assert_eq!(open(api_key, OLD_KEY), None);
    assert_eq!(values["retries"], JsonData::Number(Number::from(3_u32)));
    assert_eq!(
        row.fields.base.credential_info.0["region"],
        text("eu-west-1")
    );
}

// credential_info is nullable in LiteLLM's schema; docs/compatibility.md reads null or absent as the empty payload, so the values still rotate.
#[rstest]
#[case::null(Some(JsonData::Null))]
#[case::absent(None)]
#[tokio::test]
async fn a_credential_row_without_info_still_rotates(#[case] info: Option<JsonData>) {
    let mut fixture = fixture();
    fixture.store.credentials = vec![stored_credential_json(info)];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    let writes = credential_writes(&fixture.store.writes());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, "c1");
    assert!(writes[0].1.fields.base.credential_info.0.is_empty());
    let values = payload_values(&writes[0].1);
    let JsonData::String(api_key) = &values["api_key"] else {
        panic!("the stored api key is text");
    };
    assert_eq!(open(api_key, NEW_KEY).as_deref(), Some("sk-live"));
}

// LiteLLM writes credential_values and credential_info as JSON objects under a null updated_by, without defaulting the actor.
#[tokio::test]
async fn a_null_actor_is_written_as_given() {
    let mut fixture = fixture();
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-live")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    let writes = credential_writes(&fixture.store.writes());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].2, None);
}

// The per-credential try/except logs one failed update and continues with the next credential.
#[tokio::test]
async fn one_failing_credential_update_does_not_stop_the_others() {
    let mut fixture = fixture();
    fixture.store.credentials = vec![
        credential_row("c1", vec![("api_key", text(&sealed("sk-1")))]),
        credential_row("c2", vec![("api_key", text(&sealed("sk-2")))]),
        credential_row("c3", vec![("api_key", text(&sealed("sk-3")))]),
    ];
    fixture.store.rejected = vec!["c2".to_owned()];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    let names: Vec<String> = credential_writes(&fixture.store.writes())
        .into_iter()
        .map(|(name, _row, _actor)| name)
        .collect();
    assert_eq!(names, vec!["c1".to_owned(), "c3".to_owned()]);
}

// LiteLLM's final debug line counts the rows it scanned, not the ones that updated.
#[tokio::test]
async fn the_credential_count_is_the_scan_count() {
    let (logs, _guard) = capture_logs();
    let mut fixture = fixture();
    fixture.store.credentials = vec![
        credential_row("c1", vec![("api_key", text(&sealed("sk-1")))]),
        credential_row("c2", vec![("api_key", text(&sealed("sk-2")))]),
        credential_row("c3", vec![("api_key", text(&sealed("sk-3")))]),
    ];
    fixture.store.rejected = vec!["c2".to_owned()];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    assert_eq!(credential_writes(&fixture.store.writes()).len(), 2);
    assert!(
        logs.contents()
            .contains("Successfully re-encrypted 3 credentials with new master key")
    );
}

// A row whose credential_values do not decode is skipped before the value codec sees it.
#[tokio::test]
async fn an_undecodable_credential_row_is_skipped() {
    let mut fixture = fixture();
    let undecodable = object(vec![
        ("credential_name", text("bad")),
        (
            "credential_values",
            JsonData::String(r#"{"api_key":"abc"}"#.to_owned()),
        ),
    ]);
    fixture.store.credentials = vec![
        credential_row("c1", vec![("api_key", text(&sealed("sk-1")))]),
        undecodable,
        credential_row("c2", vec![("api_key", text(&sealed("sk-2")))]),
    ];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    let names: Vec<String> = credential_writes(&fixture.store.writes())
        .into_iter()
        .map(|(name, _row, _actor)| name)
        .collect();
    assert_eq!(names, vec!["c1".to_owned(), "c2".to_owned()]);
}

#[tokio::test]
async fn a_credential_scan_failure_skips_only_the_credential_stage() {
    let mut fixture = fixture();
    fixture.store.failures = vec![Failure::CredentialScan];
    fixture.store.models = vec![model_row(
        "m1",
        vec![("model", text(&sealed("openai/gpt-4o")))],
    )];
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-live")))],
    )];
    fixture.rotation().run(None).await.unwrap();
    assert_eq!(saved_models(&fixture.store.writes()).len(), 1);
    assert!(credential_writes(&fixture.store.writes()).is_empty());
}

// decrypt_credentials keeps the stored text for a value it cannot open and update_db_credential seals that text again, so LiteLLM stacks a second layer; kept on purpose.
#[tokio::test]
async fn an_unreadable_credential_value_is_sealed_again() {
    let mut fixture = fixture();
    let unreadable = sealed_by("another-master-key", "sk-lost");
    fixture.store.credentials = vec![credential_row("c1", vec![("api_key", text(&unreadable))])];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    let writes = credential_writes(&fixture.store.writes());
    assert_eq!(writes.len(), 1);
    let values = payload_values(&writes[0].1);
    let JsonData::String(api_key) = &values["api_key"] else {
        panic!("the stored api key is text");
    };
    assert_eq!(open(api_key, NEW_KEY), Some(unreadable));
    assert_eq!(open(api_key, OLD_KEY), None);
}

// LiteLLM's `if credentials:` skips the loop and the final debug line, so an empty scan logs no count.
#[tokio::test]
async fn an_empty_credential_scan_writes_nothing() {
    let (logs, _guard) = capture_logs();
    let mut fixture = fixture();
    fixture.rotation().run(None).await.unwrap();
    assert!(credential_writes(&fixture.store.writes()).is_empty());
    assert!(!logs.contents().contains("Successfully re-encrypted"));
}

// The per-credential try/except swallows a failed update; the rotation still finishes.
#[tokio::test]
async fn a_credential_write_failure_is_not_an_error() {
    let mut fixture = fixture();
    fixture.store.credentials = vec![credential_row(
        "c1",
        vec![("api_key", text(&sealed("sk-live")))],
    )];
    fixture.store.rejected = vec!["c1".to_owned()];
    fixture.rotation().run(Some("user-7")).await.unwrap();
    assert!(credential_writes(&fixture.store.writes()).is_empty());
}
