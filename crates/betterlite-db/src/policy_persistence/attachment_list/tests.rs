#![cfg(test)]

use betterlite_policy::row_adapter::{PolicyOrder, PolicySort};

use super::get_all_attachments_from_db;
use crate::policy_persistence::tests::{Call, Fixture, attachment_row};

#[tokio::test]
async fn lists_every_attachment_newest_first() {
    let rows = [
        attachment_row("old", "a", "2026-01-01T00:00:00Z"),
        attachment_row("new", "b", "2026-01-03T00:00:00Z"),
        attachment_row("mid", "c", "2026-01-02T00:00:00Z"),
    ];
    let fixture = Fixture::new(&[], &rows);
    let listed = get_all_attachments_from_db(&fixture.attachment_store())
        .await
        .unwrap();
    let ids: Vec<(&str, &str)> = listed
        .iter()
        .map(|found| (found.attachment_id.as_str(), found.policy_name.as_str()))
        .collect();
    assert_eq!(ids, [("new", "b"), ("mid", "c"), ("old", "a")]);
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    assert_eq!(
        fixture.client.attachments.calls(),
        [Call::FindMany(None, Some(order))]
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

#[tokio::test]
async fn wraps_a_failed_listing() {
    let fixture = Fixture::new(&[], &[]);
    fixture.client.attachments.fail("find_many");
    let error = get_all_attachments_from_db(&fixture.attachment_store())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting attachments from DB: table refused find_many"
    );
}
