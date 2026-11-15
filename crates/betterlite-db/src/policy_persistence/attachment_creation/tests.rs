#![cfg(test)]

use betterlite_policy::records::attachment::PolicyAttachmentCreateRequest;
use betterlite_policy::values::PolicyAttachment;

use super::add_attachment_to_db;
use crate::policy_persistence::tests::{Fixture, NOW};

#[tokio::test]
async fn inserts_the_row_and_registers_the_attachment() {
    let fixture = Fixture::new(&[], &[]);
    let request: PolicyAttachmentCreateRequest = serde_json::from_str(
        r#"{"policy_name": "pii", "teams": ["team-a"], "tags": [], "priority": 2, "default": true}"#,
    )
    .unwrap();
    let response = add_attachment_to_db(&fixture.attachment_store(), &request, Some("admin"))
        .await
        .unwrap();
    let stored = format!(
        r#"{{"attachment_id":"attachment_id-0","created_at":"{NOW}","created_by":"admin","is_default":true,"keys":[],"models":[],"policy_name":"pii","priority":2,"scope":null,"tags":[],"teams":["team-a"],"updated_at":"{NOW}","updated_by":"admin"}}"#
    );
    assert_eq!(
        serde_json::to_string(&fixture.client.attachments.rows()).unwrap(),
        format!("[{stored}]")
    );
    let returned = format!(
        r#"{{"attachment_id":"attachment_id-0","policy_name":"pii","scope":null,"teams":["team-a"],"keys":[],"models":[],"tags":[],"priority":2,"default":true,"created_at":"{NOW}","updated_at":"{NOW}","created_by":"admin","updated_by":"admin","definition_location":"db"}}"#
    );
    assert_eq!(serde_json::to_string(&response).unwrap(), returned);
    let registry = fixture.attachments.into_inner().unwrap();
    let registered = PolicyAttachment {
        policy: "pii".to_owned(),
        scope: None,
        teams: Some(vec!["team-a".to_owned()]),
        keys: None,
        models: None,
        tags: Some(Vec::new()),
        priority: Some(2),
        default: true,
    };
    assert_eq!(
        (registry.attachments, registry.initialized),
        (vec![registered], true)
    );
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
    assert!(
        fixture
            .attachments
            .into_inner()
            .unwrap()
            .attachments
            .is_empty()
    );
}
