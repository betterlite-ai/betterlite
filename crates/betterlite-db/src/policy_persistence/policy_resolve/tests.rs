#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_policy::row_adapter::{PolicyOrder, PolicySort};

use super::resolve_guardrails_from_db;
use crate::policy_persistence::tests::{Call, Fixture, id_filter, object, policy_row};

fn fixture() -> Fixture {
    let mut base = policy_row("p1", "base", "production", "2026-01-01T00:00:00Z");
    base.guardrails_add = Some(vec!["base-guard".to_owned(), "base-drop".to_owned()]);
    let mut child = policy_row("p2", "child", "production", "2026-01-02T00:00:00Z");
    child.inherit = Some("base".to_owned());
    child.guardrails_add = Some(vec!["zeta".to_owned(), "alpha".to_owned()]);
    child.guardrails_remove = Some(vec!["base-drop".to_owned()]);
    child.condition = Some(object(&BTreeMap::from([("model", "never-matches")])));
    let mut draft = policy_row("p3", "child", "draft", "2026-01-03T00:00:00Z");
    draft.guardrails_add = Some(vec!["draft-only".to_owned()]);
    Fixture::new(&[base, child, draft], &[])
}

#[tokio::test]
async fn resolves_inheritance_against_production_versions_only() {
    let fixture = fixture();
    let guardrails = resolve_guardrails_from_db(&fixture.policy_store(), "child")
        .await
        .unwrap();
    assert_eq!(guardrails, ["alpha", "base-guard", "zeta"]);
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let filter = id_filter("version_status", "production");
    assert_eq!(
        fixture.client.policies.calls(),
        [Call::FindMany(Some(filter), Some(order))]
    );
    assert!(fixture.policies.into_inner().unwrap().policies.is_empty());
}

#[tokio::test]
async fn an_unknown_policy_resolves_to_nothing() {
    let fixture = fixture();
    let guardrails = resolve_guardrails_from_db(&fixture.policy_store(), "missing")
        .await
        .unwrap();
    assert!(guardrails.is_empty());
}

#[tokio::test]
async fn a_failed_listing_names_both_operations() {
    let fixture = fixture();
    fixture.client.policies.fail("find_many");
    let error = resolve_guardrails_from_db(&fixture.policy_store(), "child")
        .await
        .unwrap_err();
    let message = "Error resolving guardrails from DB: Error getting policies from DB: table refused find_many";
    assert_eq!(error.to_string(), message);
}
