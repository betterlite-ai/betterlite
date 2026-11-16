#![cfg(test)]

use super::get_attachment_by_id_from_db;
use crate::policy_persistence::tests::{Call, Fixture, attachment_row, id_filter};

#[tokio::test]
async fn returns_the_stored_attachment_or_none() {
    let mut row = attachment_row("a1", "pii", "2026-01-01T00:00:00Z");
    row.scope = Some("*".to_owned());
    row.priority = Some(-1_i32);
    row.is_default = true;
    row.created_by = Some("admin".to_owned());
    row.updated_by = Some("editor".to_owned());
    let fixture = Fixture::new(&[], &[row]);
    let found = get_attachment_by_id_from_db(&fixture.attachment_store(), "a1")
        .await
        .unwrap();
    let expected = r#"{"attachment_id":"a1","policy_name":"pii","scope":"*","teams":["team-a"],"keys":[],"models":[],"tags":[],"priority":-1,"default":true,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","created_by":"admin","updated_by":"editor","definition_location":"db"}"#;
    assert_eq!(serde_json::to_string(&found).unwrap(), expected);
    let missing = get_attachment_by_id_from_db(&fixture.attachment_store(), "a2")
        .await
        .unwrap();
    assert_eq!(missing, None);
    let lookups = [
        Call::FindUnique(id_filter("attachment_id", "a1")),
        Call::FindUnique(id_filter("attachment_id", "a2")),
    ];
    assert_eq!(fixture.client.attachments.calls(), lookups);
}

#[tokio::test]
async fn wraps_a_failed_lookup() {
    let fixture = Fixture::new(&[], &[]);
    fixture.client.attachments.fail("find_unique");
    let error = get_attachment_by_id_from_db(&fixture.attachment_store(), "a1")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting attachment from DB: table refused find_unique"
    );
}
