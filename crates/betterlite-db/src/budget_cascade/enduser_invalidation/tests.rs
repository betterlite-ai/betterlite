#![cfg(test)]

use std::collections::BTreeMap;

use deadpool_postgres::Client;
use rust_decimal::Decimal;

use super::{EndUserBudgetMatch, enduser_invalidation_match, queue_enduser_resets};
use crate::budget_cascade::tests::{
    Database, RecordedWrite, RecordingWrites, cascade, seed_linked_rows, spend_of,
};
use crate::budget_cascade::transaction::commit_budget_cascade;
use crate::budget_reset_policy::filters::SPENT_ROWS;
use crate::spend_reset_unit_of_work::queue::{BudgetIdFilter, LinkedResetFilter, SpendRange};

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

const fn null_filter(spend: SpendRange) -> LinkedResetFilter {
    LinkedResetFilter {
        budget_id: BudgetIdFilter::IsNull,
        spend,
    }
}

#[test]
fn linked_customers_reset_by_budget_link_only() {
    let collected = cascade(&["budget-a", "budget-b"], BTreeMap::new());
    let writes = RecordingWrites::default();
    queue_enduser_resets(&writes, &collected, None);
    assert_eq!(
        writes.writes(),
        vec![RecordedWrite::Zero(LinkedResetFilter {
            budget_id: BudgetIdFilter::In(ids(&["budget-a", "budget-b"])),
            spend: SPENT_ROWS,
        })]
    );
}

#[test]
fn null_customers_join_when_the_default_tier_is_expiring() {
    let collected = cascade(&["budget-a"], BTreeMap::new());
    let writes = RecordingWrites::default();
    queue_enduser_resets(&writes, &collected, Some("budget-a"));
    assert_eq!(
        writes.writes(),
        vec![
            RecordedWrite::Zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::In(ids(&["budget-a"])),
                spend: SPENT_ROWS,
            }),
            RecordedWrite::Zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::IsNull,
                spend: SPENT_ROWS,
            }),
        ]
    );
}

#[test]
fn null_customers_stay_out_when_the_default_tier_is_not_expiring() {
    let collected = cascade(&["budget-a"], BTreeMap::new());
    let writes = RecordingWrites::default();
    queue_enduser_resets(&writes, &collected, Some("budget-b"));
    assert_eq!(writes.writes().len(), 1);
    assert_eq!(
        writes.writes()[0],
        RecordedWrite::Zero(LinkedResetFilter {
            budget_id: BudgetIdFilter::In(ids(&["budget-a"])),
            spend: SPENT_ROWS,
        })
    );
}

#[test]
fn null_customers_carry_over_the_default_tier_cap() {
    let limit = Decimal::from(20_i32);
    let collected = cascade(
        &["budget-a"],
        BTreeMap::from([("budget-a".to_owned(), limit)]),
    );
    let writes = RecordingWrites::default();
    queue_enduser_resets(&writes, &collected, Some("budget-a"));
    let recorded = writes.writes();
    assert_eq!(recorded.len(), 4);
    assert!(matches!(&recorded[0], RecordedWrite::Zero(_)));
    assert!(matches!(&recorded[1], RecordedWrite::Decrement(_, _)));
    assert_eq!(
        recorded[2],
        RecordedWrite::Zero(null_filter(SpendRange {
            greater_than: Some(Decimal::ZERO),
            less_than_or_equal: Some(limit),
        }))
    );
    assert_eq!(
        recorded[3],
        RecordedWrite::Decrement(
            null_filter(SpendRange {
                greater_than: Some(limit),
                less_than_or_equal: None,
            }),
            limit,
        )
    );
}

#[test]
fn match_reports_whether_the_default_tier_is_expiring() {
    let budget_ids = ids(&["budget-a", "budget-b"]);
    let matching = enduser_invalidation_match(&budget_ids, Some("budget-b"));
    assert_eq!(
        matching,
        EndUserBudgetMatch {
            linked_budget_ids: budget_ids.as_slice(),
            include_unlinked: true,
        }
    );
    let other = enduser_invalidation_match(&budget_ids, Some("budget-c"));
    assert!(!other.include_unlinked);
    assert!(!enduser_invalidation_match(&budget_ids, None).include_unlinked);
}

async fn assert_customer_spend(client: &Client, user_id: &str, expected: Decimal) {
    assert_eq!(
        spend_of(client, "LiteLLM_EndUserTable", "user_id", user_id).await,
        expected
    );
}

#[tokio::test]
async fn null_customers_reset_with_the_default_tier() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    database
        .client
        .batch_execute(
            "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id) VALUES ('budget-b'); INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('customer-b', 'budget-b', 10);",
        )
        .await
        .expect("extra customers seed");
    let collected = cascade(&["budget-a"], BTreeMap::new());
    commit_budget_cascade(&database.store, &collected, Some("budget-a"))
        .await
        .expect("cascade commits");
    assert_customer_spend(&database.client, "customer-linked", Decimal::ZERO).await;
    assert_customer_spend(&database.client, "customer-null", Decimal::ZERO).await;
    assert_customer_spend(&database.client, "customer-b", Decimal::from(10_i32)).await;
    database.finish().await;
}

#[tokio::test]
async fn null_customers_keep_spend_when_the_default_tier_is_not_expiring() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let collected = cascade(&["budget-a"], BTreeMap::new());
    commit_budget_cascade(&database.store, &collected, Some("budget-b"))
        .await
        .expect("cascade commits");
    assert_customer_spend(&database.client, "customer-null", Decimal::from(10_i32)).await;
    assert_customer_spend(&database.client, "customer-linked", Decimal::ZERO).await;
    database.finish().await;
}
