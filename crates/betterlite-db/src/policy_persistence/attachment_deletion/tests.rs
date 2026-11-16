#![cfg(test)]

use betterlite_policy::values::PolicyAttachment;
use rstest::rstest;

use super::delete_attachment_from_db;
use crate::policy_persistence::Deletion;
use crate::policy_persistence::tests::{Call, Fixture, attachment_row, id_filter};

fn attached(name: &str) -> PolicyAttachment {
    let teams = Some(vec!["team-a".to_owned()]);
    let policy = name.to_owned();
    PolicyAttachment {
        policy,
        scope: None,
        teams,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn fixture() -> Fixture {
    let rows = [
        attachment_row("a1", "pii", "2026-01-01T00:00:00Z"),
        attachment_row("a2", "other", "2026-01-02T00:00:00Z"),
    ];
    let fixture = Fixture::new(&[], &rows);
    let mut registry = fixture.attachments.lock().unwrap();
    registry.load_attachments(vec![attached("config")]);
    registry.add_attachment(attached("pii"));
    registry.initialized = false;
    drop(registry);
    fixture
}

#[tokio::test]
async fn deletes_the_row_and_rebuilds_the_registry_from_the_table() {
    let fixture = fixture();
    let deleted = delete_attachment_from_db(&fixture.attachment_store(), "a1")
        .await
        .unwrap();
    assert_eq!(
        deleted,
        Deletion {
            message: "Attachment a1 deleted successfully".to_owned(),
            warning: None
        }
    );
    assert_eq!(
        fixture.client.attachments.calls().get(1),
        Some(&Call::Delete(id_filter("attachment_id", "a1")))
    );
    let registry = fixture.attachments.into_inner().unwrap();
    assert_eq!(
        (registry.attachments, registry.initialized),
        (vec![attached("config"), attached("other")], true)
    );
}

#[rstest]
#[case::unknown_id(
    "nope",
    "find_many",
    "Error deleting attachment from DB: Attachment with ID nope not found"
)]
#[case::failed_delete(
    "a1",
    "delete",
    "Error deleting attachment from DB: table refused delete"
)]
#[case::failed_resync(
    "a1",
    "find_many",
    "Error deleting attachment from DB: Error syncing attachments from DB: Error getting attachments from DB: table refused find_many"
)]
#[tokio::test]
async fn reports_every_failure_with_its_operations(
    #[case] attachment_id: &str,
    #[case] failing: &'static str,
    #[case] message: &str,
) {
    let fixture = fixture();
    fixture.client.attachments.fail(failing);
    let error = delete_attachment_from_db(&fixture.attachment_store(), attachment_id)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert_eq!(
        fixture.attachments.into_inner().unwrap().attachments.len(),
        2
    );
}
