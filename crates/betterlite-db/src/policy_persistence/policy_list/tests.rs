#![cfg(test)]

use betterlite_policy::registry::PolicySource;
use betterlite_policy::row_adapter::{PolicyOrder, PolicySort};
use rstest::rstest;

use super::get_all_policies_from_db;
use crate::policy_persistence::tests::{Call, Fixture, id_filter, policy_row};

fn fixture() -> Fixture {
    let rows = [
        policy_row("old", "a", "production", "2026-01-01T00:00:00Z"),
        policy_row("mid", "b", "draft", "2026-01-02T00:00:00Z"),
        policy_row("new", "c", "production", "2026-01-03T00:00:00Z"),
    ];
    Fixture::new(&rows, &[])
}

#[rstest]
#[case::production_only(Some("production"), &["new", "old"])]
#[case::draft_only(Some("draft"), &["mid"])]
#[case::every_status(None, &["new", "mid", "old"])]
#[tokio::test]
async fn filters_by_status_and_lists_newest_first(
    #[case] status: Option<&str>,
    #[case] expected: &[&str],
) {
    let fixture = fixture();
    let listed = get_all_policies_from_db(&fixture.policy_store(), status)
        .await
        .unwrap();
    let ids: Vec<&str> = listed
        .iter()
        .map(|policy| policy.policy_id.as_str())
        .collect();
    assert_eq!(ids, expected);
    assert!(
        listed
            .iter()
            .all(|policy| policy.definition_location == PolicySource::Db)
    );
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let filter = status.map(|wanted| id_filter("version_status", wanted));
    assert_eq!(
        fixture.client.policies.calls(),
        [Call::FindMany(filter, Some(order))]
    );
}

#[tokio::test]
async fn wraps_a_failed_listing() {
    let fixture = fixture();
    fixture.client.policies.fail("find_many");
    let error = get_all_policies_from_db(&fixture.policy_store(), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting policies from DB: table refused find_many"
    );
}
