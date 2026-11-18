#![cfg(test)]

use super::super::linked_rows::{LinkedKeys as _, OrganizationRow};

fn row() -> OrganizationRow {
    OrganizationRow {
        organization_id: "org-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn org_counter_key_wraps_the_organization_under_spend_org() {
    assert_eq!(row().counter_key(), "spend:org:org-a".to_owned());
}

#[test]
fn org_cache_keys_include_the_plain_and_with_budget_entries() {
    assert_eq!(
        row().cache_keys(),
        vec![
            "org_id:org-a".to_owned(),
            "org_id:org-a:with_budget".to_owned()
        ]
    );
}
