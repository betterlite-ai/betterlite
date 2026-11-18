#![cfg(test)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use rust_decimal::Decimal;

use super::commit_budget_cascade;
use crate::budget_cascade::tests::{
    Database, cascade, reject_budget_updates, reset_at_of, seed_linked_rows, spend_of,
};
use crate::budget_cascade::{BudgetCascade, BudgetReset};
use crate::postgres_store::session::PostgresStore;

const LINKED_TARGETS: [(&str, &str, &str); 7] = [
    ("LiteLLM_TeamMembership", "user_id", "user-a"),
    ("LiteLLM_VerificationToken", "token", "token-a"),
    ("LiteLLM_OrganizationTable", "organization_id", "org-a"),
    ("LiteLLM_TagTable", "tag_name", "tag-a"),
    (
        "LiteLLM_ModelAccessGroupBudgetTable",
        "access_group_name",
        "group-a",
    ),
    ("LiteLLM_ProjectTable", "project_id", "project-a"),
    ("LiteLLM_EndUserTable", "user_id", "customer-linked"),
];

fn at() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp is valid")
}

async fn assert_linked_spends(client: &Client, expected: Decimal) {
    for (table, column, id) in LINKED_TARGETS {
        assert_eq!(
            spend_of(client, table, column, id).await,
            expected,
            "{table} rows linked to the tier hold the expected spend"
        );
    }
}

#[tokio::test]
async fn commit_zeroes_linked_rows_and_advances_every_window() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    let mut collected = cascade(&["budget-a"], BTreeMap::new());
    collected.budget_resets.push(BudgetReset {
        budget_id: "budget-a".to_owned(),
        reset_at: at(),
    });
    commit_budget_cascade(&database.store, &collected, None)
        .await
        .expect("cascade commits");
    assert_linked_spends(&database.client, Decimal::ZERO).await;
    assert_eq!(
        spend_of(
            &database.client,
            "LiteLLM_VerificationToken",
            "token",
            "token-window"
        )
        .await,
        Decimal::from(10_i32)
    );
    assert_eq!(
        spend_of(
            &database.client,
            "LiteLLM_EndUserTable",
            "user_id",
            "customer-null"
        )
        .await,
        Decimal::from(10_i32)
    );
    assert_eq!(
        reset_at_of(&database.client, "budget-a").await,
        Some(at().naive_utc())
    );
    database.finish().await;
}

#[tokio::test]
async fn commit_rolls_back_every_table_when_the_window_advance_fails() {
    let database = Database::provision().await;
    seed_linked_rows(&database.client).await;
    reject_budget_updates(&database.client).await;
    let mut collected = cascade(&["budget-a"], BTreeMap::new());
    collected.budget_resets.push(BudgetReset {
        budget_id: "budget-a".to_owned(),
        reset_at: at(),
    });
    let outcome = commit_budget_cascade(&database.store, &collected, None).await;
    assert!(outcome.is_err());
    assert_linked_spends(&database.client, Decimal::from(10_i32)).await;
    assert_eq!(reset_at_of(&database.client, "budget-a").await, None);
    database.finish().await;
}

#[tokio::test]
async fn commit_with_no_due_tiers_touches_no_database() {
    let store = PostgresStore { pool: None };
    commit_budget_cascade(&store, &BudgetCascade::default(), None)
        .await
        .expect("nothing to commit");
}
