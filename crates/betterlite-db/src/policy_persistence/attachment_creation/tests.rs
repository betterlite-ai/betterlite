#![cfg(test)]

use betterlite_policy::records::attachment::PolicyAttachmentCreateRequest;
use rstest::rstest;

use super::add_attachment_to_db;
use crate::policy_persistence::tests::{Fixture, NOW};

const FULL_REQUEST: &str = r#"{"policy_name": "pii", "scope": "*", "teams": ["team-a"], "keys": ["key-a"], "models": ["gpt-4"], "tags": ["prod"], "priority": 2, "default": true}"#;

const STORED_FULL: &str = r#"{"attachment_id":"attachment_id-0","created_at":"NOW","created_by":"admin","is_default":true,"keys":["key-a"],"models":["gpt-4"],"policy_name":"pii","priority":2,"scope":"*","tags":["prod"],"teams":["team-a"],"updated_at":"NOW","updated_by":"admin"}"#;

const RETURNED_FULL: &str = r#"{"attachment_id":"attachment_id-0","policy_name":"pii","scope":"*","teams":["team-a"],"keys":["key-a"],"models":["gpt-4"],"tags":["prod"],"priority":2,"default":true,"created_at":"NOW","updated_at":"NOW","created_by":"admin","updated_by":"admin","definition_location":"db"}"#;

const REGISTERED_FULL: &str = r#"[{"policy":"pii","scope":"*","teams":["team-a"],"keys":["key-a"],"models":["gpt-4"],"tags":["prod"],"priority":2,"default":true}]"#;

const STORED_BARE: &str = r#"{"attachment_id":"attachment_id-0","created_at":"NOW","created_by":null,"is_default":false,"keys":[],"models":[],"policy_name":"pii","priority":null,"scope":null,"tags":[],"teams":[],"updated_at":"NOW","updated_by":null}"#;

const RETURNED_BARE: &str = r#"{"attachment_id":"attachment_id-0","policy_name":"pii","scope":null,"teams":[],"keys":[],"models":[],"tags":[],"priority":null,"default":false,"created_at":"NOW","updated_at":"NOW","created_by":null,"updated_by":null,"definition_location":"db"}"#;

const REGISTERED_BARE: &str = r#"[{"policy":"pii","scope":null,"teams":null,"keys":null,"models":null,"tags":null,"priority":null,"default":false}]"#;

#[rstest]
#[case::full(
    FULL_REQUEST,
    Some("admin"),
    STORED_FULL,
    RETURNED_FULL,
    REGISTERED_FULL
)]
#[case::bare(
    r#"{"policy_name": "pii"}"#,
    None,
    STORED_BARE,
    RETURNED_BARE,
    REGISTERED_BARE
)]
#[tokio::test]
async fn inserts_the_row_and_registers_the_attachment(
    #[case] body: &str,
    #[case] created_by: Option<&str>,
    #[case] stored: &str,
    #[case] returned: &str,
    #[case] registered: &str,
) {
    let fixture = Fixture::new(&[], &[]);
    let request: PolicyAttachmentCreateRequest = serde_json::from_str(body).unwrap();
    let response = add_attachment_to_db(&fixture.attachment_store(), &request, created_by)
        .await
        .unwrap();
    let rows = serde_json::to_string(&fixture.client.attachments.rows()).unwrap();
    assert_eq!(rows, format!("[{}]", stored.replace("NOW", NOW)));
    let response_text = serde_json::to_string(&response).unwrap();
    assert_eq!(response_text, returned.replace("NOW", NOW));
    let registry = fixture.attachments.into_inner().unwrap();
    assert_eq!(
        serde_json::to_string(&registry.attachments).unwrap(),
        registered
    );
    assert!(registry.initialized);
}

#[tokio::test]
async fn wraps_a_failed_insert_and_registers_nothing() {
    let fixture = Fixture::new(&[], &[]);
    fixture.client.attachments.fail("create");
    let request: PolicyAttachmentCreateRequest =
        serde_json::from_str(r#"{"policy_name": "pii", "scope": "*"}"#).unwrap();
    let error = add_attachment_to_db(&fixture.attachment_store(), &request, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error adding attachment to DB: table refused create"
    );
    let registry = fixture.attachments.into_inner().unwrap();
    assert!(registry.attachments.is_empty());
}
