#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_analytics::daily_metrics::BreakdownMetrics;
use indexmap::IndexMap;

use super::dispatch::{
    aggregate_grouping_rows_sync, assign_api_key_breakdown, assign_metric_with_metadata,
    ensure_date,
};
use super::levels::{GROUP_DATE, GROUP_DATE_MODEL, GROUP_GRAND_TOTAL};
use super::support::{
    DAY_ONE, DAY_TWO, KEY_ONE, KEY_TWO, RowParts, amount, key_map, key_metadata_for, key_row,
    missing_key_metadata, row, zero_metrics,
};

#[test]
fn ensure_date_creates_a_missing_bucket() {
    let mut grouped = BTreeMap::new();
    let bucket = ensure_date(&mut grouped, DAY_ONE);
    assert_eq!(bucket.metrics, zero_metrics());
    assert_eq!(bucket.breakdown, BreakdownMetrics::default());
    assert!(grouped.contains_key(DAY_ONE));
}

#[test]
fn ensure_date_returns_the_existing_bucket() {
    let mut grouped = BTreeMap::new();
    ensure_date(&mut grouped, DAY_ONE).metrics.spend = amount(5);
    let bucket = ensure_date(&mut grouped, DAY_ONE);
    assert_eq!(bucket.metrics.spend, amount(5));
    assert_eq!(grouped.len(), 1);
}

#[test]
fn ensure_date_keeps_distinct_dates_apart() {
    let mut grouped = BTreeMap::new();
    ensure_date(&mut grouped, DAY_ONE).metrics.spend = amount(1);
    ensure_date(&mut grouped, DAY_TWO).metrics.spend = amount(2);
    assert_eq!(grouped.len(), 2);
    assert_eq!(ensure_date(&mut grouped, DAY_ONE).metrics.spend, amount(1));
}

#[test]
fn assign_metric_with_metadata_inserts_a_new_bucket() {
    let mut target = IndexMap::new();
    let mut metrics = zero_metrics();
    metrics.spend = amount(5);
    assign_metric_with_metadata(&mut target, "gpt-4", metrics);
    let bucket = target.get("gpt-4").unwrap();
    assert_eq!(bucket.metrics.spend, amount(5));
    assert!(bucket.metadata.is_empty());
    assert!(bucket.api_key_breakdown.is_empty());
}

#[test]
fn assign_metric_with_metadata_replaces_metrics_on_an_existing_bucket() {
    let mut target = IndexMap::new();
    let mut key_metrics = zero_metrics();
    key_metrics.spend = amount(2);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, key_metrics, &BTreeMap::new());
    let mut parent_metrics = zero_metrics();
    parent_metrics.spend = amount(9);
    assign_metric_with_metadata(&mut target, "gpt-4", parent_metrics);
    let parent = target.get("gpt-4").unwrap();
    assert_eq!(parent.metrics.spend, amount(9));
    assert!(parent.api_key_breakdown.contains_key(KEY_ONE));
}

#[test]
fn assign_metric_with_metadata_reuses_one_entry_per_name() {
    let mut target = IndexMap::new();
    let mut first = zero_metrics();
    first.spend = amount(1);
    let mut second = zero_metrics();
    second.spend = amount(2);
    assign_metric_with_metadata(&mut target, "gpt-4", first);
    assign_metric_with_metadata(&mut target, "gpt-4", second);
    assert_eq!(target.len(), 1);
    assert_eq!(target.get("gpt-4").unwrap().metrics.spend, amount(2));
}

#[test]
fn assign_api_key_breakdown_creates_the_parent_with_zero_metrics() {
    let mut target = IndexMap::new();
    let mut metrics = zero_metrics();
    metrics.spend = amount(5);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, metrics, &BTreeMap::new());
    let parent = target.get("gpt-4").unwrap();
    assert_eq!(parent.metrics, zero_metrics());
    assert!(parent.metadata.is_empty());
    let key = parent.api_key_breakdown.get(KEY_ONE).unwrap();
    assert_eq!(key.metrics.spend, amount(5));
    assert_eq!(key.metadata, missing_key_metadata());
}

#[test]
fn assign_api_key_breakdown_keeps_parent_metrics() {
    let mut target = IndexMap::new();
    let mut parent_metrics = zero_metrics();
    parent_metrics.spend = amount(9);
    assign_metric_with_metadata(&mut target, "gpt-4", parent_metrics);
    let mut key_metrics = zero_metrics();
    key_metrics.spend = amount(5);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, key_metrics, &BTreeMap::new());
    assert_eq!(target.get("gpt-4").unwrap().metrics.spend, amount(9));
}

#[test]
fn assign_api_key_breakdown_adds_each_key() {
    let mut target = IndexMap::new();
    let mut first = zero_metrics();
    first.spend = amount(1);
    let mut second = zero_metrics();
    second.spend = amount(2);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, first, &BTreeMap::new());
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_TWO, second, &BTreeMap::new());
    let parent = target.get("gpt-4").unwrap();
    assert_eq!(parent.api_key_breakdown.len(), 2);
    assert_eq!(
        parent.api_key_breakdown.get(KEY_TWO).unwrap().metrics.spend,
        amount(2)
    );
}

#[test]
fn assign_api_key_breakdown_replaces_one_keys_metrics() {
    let mut target = IndexMap::new();
    let mut first = zero_metrics();
    first.spend = amount(1);
    let mut second = zero_metrics();
    second.spend = amount(2);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, first, &BTreeMap::new());
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, second, &BTreeMap::new());
    let parent = target.get("gpt-4").unwrap();
    assert_eq!(parent.api_key_breakdown.len(), 1);
    assert_eq!(
        parent.api_key_breakdown.get(KEY_ONE).unwrap().metrics.spend,
        amount(2)
    );
}

#[test]
fn assign_api_key_breakdown_attaches_known_key_metadata() {
    let mut target = IndexMap::new();
    let metadata = key_map(&[key_row(KEY_ONE, true)]);
    assign_api_key_breakdown(&mut target, "gpt-4", KEY_ONE, zero_metrics(), &metadata);
    let key = target
        .get("gpt-4")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metadata, key_metadata_for(KEY_ONE, true));
}

#[test]
fn only_a_grand_total_row_creates_no_date_bucket() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_GRAND_TOTAL, None)
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(aggregated.results.is_empty());
    assert_eq!(aggregated.totals.spend, amount(5));
}

#[test]
fn a_date_row_creates_its_bucket_before_dispatch() {
    let records = [row(RowParts {
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results.len(), 1);
    assert!(aggregated.results[0].breakdown.models.is_empty());
}

#[test]
fn a_date_row_with_metrics_creates_the_bucket_with_those_metrics() {
    let records = [row(RowParts {
        spend: Some(5),
        ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results[0].metrics.spend, amount(5));
}
