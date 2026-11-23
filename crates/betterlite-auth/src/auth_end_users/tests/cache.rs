#![cfg(test)]

use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;

use super::super::cache::{EndUserCache as _, EndUserCacheKey, options};
use super::support::{budget_fixture, test_cache};

#[test]
fn written_rows_carry_the_management_ttl() {
    let written = options(33.0_f64);
    assert_eq!(
        written.ttl.map(|ttl| ttl.seconds.to_bits()),
        Some(33.0_f64.to_bits())
    );
    assert!(!written.local_only);
    assert!(!written.refresh_ttl);
}

#[tokio::test]
async fn gateway_cache_rows_round_trip() {
    let cache = test_cache();
    let key = EndUserCacheKey::DefaultEndUserBudget("budget-1").render();
    assert!(
        cache
            .read_row::<LiteLLMBudgetTable>(&key)
            .await
            .expect("the empty read")
            .is_none()
    );
    let budget = budget_fixture("budget-1", Some("100"));
    cache
        .write_row(&key, &budget, 45.0_f64)
        .await
        .expect("the row write");
    let read = cache
        .read_row::<LiteLLMBudgetTable>(&key)
        .await
        .expect("the cached read")
        .expect("the written row");
    assert_eq!(read, budget);
}
