#![cfg(test)]

use betterlite_analytics::daily_metrics::SpendMetrics;
use chrono::NaiveDate;
use std::collections::BTreeMap;

use super::aggregate::aggregate_spend_records_sync;
use super::error::PagesError;
use super::support::{day, entity_metadata, fields, key_row, money, spend_record, text};

#[test]
fn records_are_grouped_by_date() {
    let mut first = spend_record("2025-03-10", "sk-a");
    first.model = Some(text("gpt-4o"));
    first.spend = money("1");
    let mut second = spend_record("2025-03-10", "sk-b");
    second.model = Some(text("gpt-4o"));
    second.spend = money("2");
    let mut third = spend_record("2025-03-11", "sk-a");
    third.spend = money("4");
    let aggregated =
        aggregate_spend_records_sync(&[first, second, third], &BTreeMap::new(), None, None)
            .unwrap();
    assert_eq!(aggregated.results.len(), 2);
    assert_eq!(aggregated.results[0].date, day(2025, 3, 11));
    assert_eq!(aggregated.results[0].metrics.spend, money("4"));
    assert_eq!(aggregated.results[1].metrics.spend, money("3"));
    assert_eq!(
        aggregated.results[1].breakdown.models["gpt-4o"]
            .metrics
            .spend,
        money("3")
    );
    assert_eq!(aggregated.results[1].breakdown.api_keys.len(), 2);
    assert!(
        aggregated.results[1].breakdown.models["gpt-4o"]
            .metadata
            .is_empty()
    );
    assert!(
        aggregated.results[1].breakdown.providers["unknown"]
            .metadata
            .is_empty()
    );
}

#[test]
fn results_are_sorted_by_descending_date() {
    let records = [
        spend_record("2025-03-09", "sk-a"),
        spend_record("2025-03-11", "sk-a"),
        spend_record("2025-03-10", "sk-a"),
    ];
    let aggregated = aggregate_spend_records_sync(&records, &BTreeMap::new(), None, None).unwrap();
    let dates: Vec<NaiveDate> = aggregated
        .results
        .iter()
        .map(|result| result.date)
        .collect();
    assert_eq!(
        dates,
        vec![day(2025, 3, 11), day(2025, 3, 10), day(2025, 3, 9)]
    );
}

#[test]
fn totals_add_every_record() {
    let mut first = spend_record("2025-03-10", "sk-a");
    first.spend = money("1.25");
    first.flat_cost = money("0.5");
    first.prompt_tokens = 10;
    first.completion_tokens = 4;
    let mut second = spend_record("2025-03-11", "sk-a");
    second.spend = money("2");
    second.prompt_tokens = 6;
    second.completion_tokens = 2;
    let aggregated =
        aggregate_spend_records_sync(&[first, second], &BTreeMap::new(), None, None).unwrap();
    assert_eq!(aggregated.totals.spend, money("3.25"));
    assert_eq!(aggregated.totals.flat_cost, money("0.5"));
    assert_eq!(aggregated.totals.total_tokens, 22);
    assert_eq!(aggregated.results[0].metrics.spend, money("2"));
    assert_eq!(aggregated.results[0].metrics.total_tokens, 8);
    assert_eq!(aggregated.results[1].metrics.total_tokens, 14);
}

#[test]
fn a_malformed_date_is_reported() {
    let record = spend_record("11/03/2025", "sk-a");
    let error = aggregate_spend_records_sync(&[record], &BTreeMap::new(), None, None).unwrap_err();
    assert!(matches!(error, PagesError::Date { value, .. } if value == "11/03/2025"));
}

#[test]
fn no_records_make_an_empty_page() {
    let aggregated = aggregate_spend_records_sync(&[], &BTreeMap::new(), None, None).unwrap();
    assert!(aggregated.results.is_empty());
    assert_eq!(aggregated.totals, SpendMetrics::default());
}

#[test]
fn api_key_metadata_reaches_the_breakdowns() {
    let metadata = BTreeMap::from([(text("sk-a"), key_row("sk-a", "alias-1"))]);
    let mut record = spend_record("2025-03-10", "sk-a");
    record.model = Some(text("gpt-4o"));
    let aggregated = aggregate_spend_records_sync(&[record], &metadata, None, None).unwrap();
    let key = &aggregated.results[0].breakdown.models["gpt-4o"].api_key_breakdown["sk-a"];
    assert_eq!(key.metadata.key_alias.as_deref(), Some("alias-1"));
}

#[test]
fn entity_metadata_reaches_every_day() {
    let metadata = entity_metadata(&[("Unassigned", fields(&[("plan", "free")]))]);
    let records = [
        spend_record("2025-03-10", "sk-a"),
        spend_record("2025-03-11", "sk-a"),
    ];
    let aggregated =
        aggregate_spend_records_sync(&records, &BTreeMap::new(), Some("user_id"), Some(&metadata))
            .unwrap();
    assert_eq!(aggregated.results.len(), 2);
    assert_eq!(
        aggregated.results[0].breakdown.entities["Unassigned"].metadata,
        fields(&[("plan", "free")])
    );
    assert_eq!(
        aggregated.results[1].breakdown.entities["Unassigned"].metadata,
        fields(&[("plan", "free")])
    );
}
