#![cfg(test)]

use std::collections::BTreeMap;

use super::dispatch::aggregate_grouping_rows_sync;
use super::levels::{
    GROUP_DATE, GROUP_DATE_API_KEY, GROUP_DATE_ENDPOINT_API_KEY, GROUP_DATE_MCP_API_KEY,
    GROUP_DATE_MODEL, GROUP_DATE_MODEL_API_KEY, GROUP_DATE_MODEL_GROUP_API_KEY,
    GROUP_DATE_PROVIDER_API_KEY, GROUP_GRAND_TOTAL,
};
use super::support::{DAY_ONE, KEY_ONE, RowParts, SENTINEL, amount, row};

#[test]
fn sentinel_api_key_row_is_skipped_in_the_api_key_breakdown() {
    let records = [row(RowParts {
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(aggregated.results[0].breakdown.api_keys.is_empty());
}

#[test]
fn regular_key_row_is_kept_in_the_api_key_breakdown() {
    let records = [row(RowParts {
        api_key: Some(KEY_ONE),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(
        aggregated.results[0]
            .breakdown
            .api_keys
            .contains_key(KEY_ONE)
    );
}

#[test]
fn sentinel_model_api_key_row_creates_no_model_breakdown() {
    let records = [row(RowParts {
        model: Some("gpt-4"),
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results.len(), 1);
    assert!(aggregated.results[0].breakdown.models.is_empty());
}

#[test]
fn sentinel_model_group_api_key_row_creates_no_model_group_breakdown() {
    let records = [row(RowParts {
        model_group: Some("chat"),
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL_GROUP_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(aggregated.results[0].breakdown.model_groups.is_empty());
}

#[test]
fn sentinel_provider_api_key_row_creates_no_provider_breakdown() {
    let records = [row(RowParts {
        provider: Some("openai"),
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_PROVIDER_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert!(aggregated.results[0].breakdown.providers.is_empty());
}

#[test]
fn sentinel_mcp_api_key_row_is_kept_in_the_mcp_breakdown() {
    let records = [row(RowParts {
        mcp: Some("github__search"),
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MCP_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let parent = aggregated.results[0]
        .breakdown
        .mcp_servers
        .get("github__search")
        .unwrap();
    assert!(parent.api_key_breakdown.contains_key(SENTINEL));
}

#[test]
fn sentinel_endpoint_api_key_row_is_kept_in_the_endpoint_breakdown() {
    let records = [row(RowParts {
        endpoint: Some("/v1/chat/completions"),
        api_key: Some(SENTINEL),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_ENDPOINT_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let parent = aggregated.results[0]
        .breakdown
        .endpoints
        .get("/v1/chat/completions")
        .unwrap();
    assert!(parent.api_key_breakdown.contains_key(SENTINEL));
}

#[test]
fn sentinel_date_row_contributes_to_the_date_metrics() {
    let records = [row(RowParts {
        api_key: Some(SENTINEL),
        spend: Some(5),
        flat_cost: Some(3),
        ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results[0].metrics.spend, amount(5));
    assert_eq!(aggregated.results[0].metrics.flat_cost, amount(3));
}

#[test]
fn sentinel_model_row_lands_in_the_model_breakdown() {
    let records = [row(RowParts {
        api_key: Some(SENTINEL),
        model: Some("gpt-4"),
        spend: Some(5),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let model = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert_eq!(model.metrics.spend, amount(5));
}

#[test]
fn sentinel_grand_total_row_sets_the_totals() {
    let records = [row(RowParts {
        api_key: Some(SENTINEL),
        spend: Some(5),
        flat_cost: Some(3),
        ..RowParts::new(GROUP_GRAND_TOTAL, None)
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.totals.spend, amount(5));
    assert_eq!(aggregated.totals.flat_cost, amount(3));
}
