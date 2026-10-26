#![cfg(test)]

use crate::management_routes::MASTER_KEY_ONLY_ROUTES;
use rstest::rstest;

#[rstest]
#[case::global_spend_reset(0, "/global/spend/reset")]
#[case::memory_usage_in_mem_cache(1, "/memory-usage-in-mem-cache")]
#[case::memory_cache_items(2, "/memory-usage-in-mem-cache-items")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(MASTER_KEY_ONLY_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(MASTER_KEY_ONLY_ROUTES.len(), 3);
}
