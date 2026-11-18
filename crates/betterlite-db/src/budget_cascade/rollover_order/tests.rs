#![cfg(test)]

use std::collections::BTreeMap;

use deadpool_postgres::Client;
use rust_decimal::Decimal;

use super::queue_budget_linked_resets;
use crate::budget_cascade::tests::{
    Database, RecordedWrite, RecordingWrites, cascade, seed_linked_rows, spend_of,
};
use crate::budget_cascade::transaction::commit_budget_cascade;
use crate::budget_reset_policy::filters::LINKED_KEYS;
use crate::spend_reset_unit_of_work::queue::{BudgetIdFilter, LinkedResetFilter, SpendRange};

fn cap(value: i32) -> Decimal {
    Decimal::from(value)
}

#[test]
fn under_cap_rows_are_zeroed_before_over_cap_rows_are_decremented() {
    let limit = cap(20);
    let collected = cascade(
        &["budget-a"],
        BTreeMap::from([("budget-a".to_owned(), limit)]),
    );
    let writes = RecordingWrites::default();
    queue_budget_linked_resets(&writes, &collected, SpendRange::default());
    assert_eq!(
        writes.writes(),
        vec![
            RecordedWrite::Zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::Equals("budget-a".to_owned()),
                spend: SpendRange {
                    greater_than: Some(Decimal::ZERO),
                    less_than_or_equal: Some(limit),
                },
            }),
            RecordedWrite::Decrement(
                LinkedResetFilter {
                    budget_id: BudgetIdFilter::Equals("budget-a".to_owned()),
                    spend: SpendRange {
                        greater_than: Some(limit),
                        less_than_or_equal: None,
                    },
                },
                limit,
            ),
        ]
    );
}

#[test]
fn plain_tiers_use_the_callers_spend_filter_in_one_zero() {
    let collected = cascade(&["budget-a", "budget-b"], BTreeMap::new());
    let writes = RecordingWrites::default();
    queue_budget_linked_resets(&writes, &collected, LINKED_KEYS);
    assert_eq!(
        writes.writes(),
        vec![RecordedWrite::Zero(LinkedResetFilter {
            budget_id: BudgetIdFilter::In(vec!["budget-a".to_owned(), "budget-b".to_owned()]),
            spend: LINKED_KEYS,
        })]
    );
}

#[test]
fn rollover_tiers_are_left_out_of_the_plain_zero() {
    let limit = cap(20);
    let collected = cascade(
        &["budget-a", "budget-b"],
        BTreeMap::from([("budget-a".to_owned(), limit)]),
    );
    let writes = RecordingWrites::default();
    queue_budget_linked_resets(&writes, &collected, SpendRange::default());
    let recorded = writes.writes();
    assert_eq!(recorded.len(), 3);
    assert!(matches!(&recorded[0], RecordedWrite::Zero(_)));
    assert!(matches!(&recorded[1], RecordedWrite::Decrement(_, _)));
    assert_eq!(
        recorded[2],
        RecordedWrite::Zero(LinkedResetFilter {
            budget_id: BudgetIdFilter::In(vec!["budget-b".to_owned()]),
            spend: SpendRange::default(),
        })
    );
}

#[test]
fn no_budget_ids_queue_nothing() {
    let collected = cascade(&[], BTreeMap::new());
    let writes = RecordingWrites::default();
    queue_budget_linked_resets(&writes, &collected, LINKED_KEYS);
    assert!(writes.writes().is_empty());
}

async fn seed_over_cap_rows(client: &Client) {
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, budget_id, spend) VALUES ('org-over', 'budget-a', 30); INSERT INTO \"LiteLLM_VerificationToken\" (token, budget_id, spend) VALUES ('token-over', 'budget-a', 30);",
        )
        .await
        .expect("over-cap rows seed");
}

async fn assert_rollover_spends(client: &Client, under: Decimal, over: Decimal) {
    assert_eq!(
        spend_of(
            client,
            "LiteLLM_OrganizationTable",
            "organization_id",
            "org-a"
        )
        .await,
        under
    );
    assert_eq!(
        spend_of(
            client,
            "LiteLLM_OrganizationTable",
            "organization_id",
            "org-over"
        )
        .await,
        over
    );
    assert_eq!(
        spend_of(client, "LiteLLM_VerificationToken", "token", "token-a").await,
        under
    );
    assert_eq!(
        spend_of(client, "LiteLLM_VerificationToken", "token", "token-over").await,
        over
    );
    assert_eq!(
        spend_of(client, "LiteLLM_VerificationToken", "token", "token-window").await,
        cap(10)
    );
}

#[tokio::test]
async fn over_cap_rows_keep_the_cap_after_the_under_cap_zero() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    seed_over_cap_rows(&database.client).await;
    let collected = cascade(
        &["budget-a"],
        BTreeMap::from([("budget-a".to_owned(), cap(20))]),
    );
    commit_budget_cascade(&database.store, &collected, None)
        .await
        .expect("cascade commits");
    assert_rollover_spends(&database.client, Decimal::ZERO, cap(10)).await;
    database.finish().await;
}
