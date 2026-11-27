#![cfg(test)]

use super::get_default_customer_budget;
use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_lookups::test_support::{
    Answer, LookupCall, RecordingLookups, capture_logs, decimal, time,
};

fn budget_table() -> BudgetTableSnapshot {
    BudgetTableSnapshot {
        max_budget: Some(decimal("25")),
        budget_reset_at: Some(time("2026-01-06T00:00:00Z")),
    }
}

#[tokio::test]
async fn a_configured_id_reads_the_budget_table() {
    let lookups = RecordingLookups {
        default_budget: Answer::Found(budget_table()),
        ..RecordingLookups::default()
    };
    let budget = get_default_customer_budget(&lookups, Some("budget-1")).await;
    assert_eq!(budget, Some(budget_table()));
    assert_eq!(
        lookups.calls(),
        vec![LookupCall::DefaultCustomerBudget {
            budget_id: "budget-1".to_owned(),
        }]
    );
}

#[tokio::test]
async fn no_configured_id_skips_the_read() {
    let lookups = RecordingLookups {
        default_budget: Answer::Found(budget_table()),
        ..RecordingLookups::default()
    };
    let budget = get_default_customer_budget(&lookups, None).await;
    assert_eq!(budget, None);
    assert!(lookups.calls().is_empty());
}

#[tokio::test]
async fn a_missing_budget_row_returns_none() {
    let lookups = RecordingLookups::default();
    let budget = get_default_customer_budget(&lookups, Some("budget-1")).await;
    assert_eq!(budget, None);
    assert_eq!(lookups.calls().len(), 1);
}

#[tokio::test]
async fn a_lookup_error_returns_none_and_logs_it() {
    let (logs, _guard) = capture_logs();
    let lookups = RecordingLookups {
        default_budget: Answer::Failed("the budget read failed"),
        ..RecordingLookups::default()
    };
    let budget = get_default_customer_budget(&lookups, Some("budget-1")).await;
    assert_eq!(budget, None);
    assert!(
        logs.text()
            .contains("Error getting default customer budget: the budget read failed")
    );
}
