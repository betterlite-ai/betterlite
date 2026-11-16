#![cfg(test)]

use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;
use std::collections::BTreeMap;

use super::aggregate::{aggregate_spend_records, aggregate_spend_records_sync};
use super::error::PagesError;
use super::record::DailySpendRecord;
use super::support::{
    at, entity_metadata, failing_repository, fields, key_row, keys, money, page, reads, repository,
    spend_record, strings, text, window,
};

fn mixed_records(date: &str) -> Vec<DailySpendRecord> {
    let mut real = spend_record(date, "sk-a");
    real.model = Some(text("gpt-4o"));
    real.spend = money("1");
    let mut sentinel = spend_record(date, PTU_SENTINEL_API_KEY);
    sentinel.model = Some(text("gpt-4o"));
    sentinel.spend = money("2");
    let mut unnamed = spend_record(date, "");
    unnamed.model = Some(text("gpt-4o"));
    unnamed.spend = money("3");
    vec![real, sentinel, unnamed]
}

#[tokio::test]
async fn fetches_metadata_for_real_keys_only() {
    let recovered = BTreeMap::from([(text("sk-a"), key_row("sk-a", "alias-1"))]);
    let repository = repository(page(Vec::new(), 0), reads(recovered));
    let mut real = spend_record("2025-03-10", "sk-a");
    real.model = Some(text("gpt-4o"));
    let records = [
        real,
        spend_record("2025-03-12", PTU_SENTINEL_API_KEY),
        spend_record("2025-03-11", ""),
    ];
    aggregate_spend_records(&repository, &records, None, None)
        .await
        .unwrap();
    let calls = repository.proxy_reads.read_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].api_keys, keys(&["sk-a"]));
    assert_eq!(
        calls[0].window,
        Some(window(at(2025, 3, 9, 0, 0), at(2025, 3, 14, 0, 0)))
    );
    assert_eq!(repository.database.token_calls(), vec![strings(&["sk-a"])]);
}

#[tokio::test]
async fn fetched_metadata_reaches_the_breakdowns() {
    let records = mixed_records("2025-03-10");
    let recovered = BTreeMap::from([(text("sk-a"), key_row("sk-a", "alias-1"))]);
    let repository = repository(page(Vec::new(), 0), reads(recovered));
    let aggregated = aggregate_spend_records(&repository, &records, None, None)
        .await
        .unwrap();
    let breakdown = &aggregated.results[0].breakdown;
    let models = &breakdown.models["gpt-4o"];
    assert_eq!(
        models.api_key_breakdown["sk-a"]
            .metadata
            .key_alias
            .as_deref(),
        Some("alias-1")
    );
    assert!(models.api_key_breakdown.contains_key(""));
    assert!(!models.api_key_breakdown.contains_key(PTU_SENTINEL_API_KEY));
    assert!(breakdown.api_keys.contains_key("sk-a"));
    assert!(breakdown.api_keys.contains_key(""));
    assert!(!breakdown.api_keys.contains_key(PTU_SENTINEL_API_KEY));
    assert_eq!(models.metrics.spend, money("6"));
}

#[tokio::test]
async fn keys_without_metadata_skip_the_fetch() {
    let mut sentinel = spend_record("2025-03-10", PTU_SENTINEL_API_KEY);
    sentinel.model = Some(text("gpt-4o"));
    sentinel.spend = money("2");
    let mut unnamed = spend_record("2025-03-10", "");
    unnamed.model = Some(text("gpt-4o"));
    unnamed.spend = money("3");
    let repository = repository(page(Vec::new(), 0), reads(BTreeMap::new()));
    let aggregated = aggregate_spend_records(&repository, &[sentinel, unnamed], None, None)
        .await
        .unwrap();
    assert!(repository.proxy_reads.read_calls().is_empty());
    assert!(repository.database.token_calls().is_empty());
    assert_eq!(aggregated.results[0].breakdown.api_keys.len(), 1);
    assert!(aggregated.results[0].breakdown.api_keys.contains_key(""));
}

#[tokio::test]
async fn async_aggregation_matches_the_sync_aggregator() {
    let records = mixed_records("2025-03-10");
    let recovered = BTreeMap::from([(text("sk-a"), key_row("sk-a", "alias-1"))]);
    let repository = repository(page(Vec::new(), 0), reads(recovered.clone()));
    let metadata = entity_metadata(&[("Unassigned", fields(&[("plan", "free")]))]);
    let aggregated =
        aggregate_spend_records(&repository, &records, Some("user_id"), Some(&metadata))
            .await
            .unwrap();
    let expected =
        aggregate_spend_records_sync(&records, &recovered, Some("user_id"), Some(&metadata))
            .unwrap();
    assert_eq!(aggregated.results, expected.results);
    assert_eq!(aggregated.totals, expected.totals);
}

#[tokio::test]
async fn metadata_failures_propagate() {
    let repository = failing_repository();
    let record = spend_record("2025-03-10", "sk-a");
    let result = aggregate_spend_records(&repository, &[record], None, None).await;
    assert!(matches!(result, Err(PagesError::Repository(_))));
}
