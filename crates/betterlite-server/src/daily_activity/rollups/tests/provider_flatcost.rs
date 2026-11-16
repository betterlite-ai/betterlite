#![cfg(test)]

use std::collections::BTreeMap;

use super::dispatch::aggregate_grouping_rows_sync;
use super::levels::{
    GROUP_DATE, GROUP_DATE_API_KEY, GROUP_DATE_ENDPOINT, GROUP_DATE_MCP, GROUP_DATE_MODEL,
    GROUP_DATE_MODEL_GROUP, GROUP_DATE_PROVIDER, GROUP_DATE_PROVIDER_API_KEY,
};
use super::support::{DAY_ONE, KEY_ONE, RowParts, SENTINEL, amount, row};

#[test]
fn provider_row_zeroes_the_flat_cost() {
    let records = [row(RowParts {
        provider: Some("openai"),
        spend: Some(5),
        flat_cost: Some(7),
        prompt_tokens: Some(3),
        ..RowParts::new(GROUP_DATE_PROVIDER, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let provider = aggregated.results[0]
        .breakdown
        .providers
        .get("openai")
        .unwrap();
    assert_eq!(provider.metrics.flat_cost, amount(0));
    assert_eq!(provider.metrics.spend, amount(5));
    assert_eq!(provider.metrics.prompt_tokens, 3);
}

#[test]
fn sentinel_provider_row_still_creates_the_unknown_bucket() {
    let records = [row(RowParts {
        api_key: Some(SENTINEL),
        spend: Some(5),
        flat_cost: Some(9),
        ..RowParts::new(GROUP_DATE_PROVIDER, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let provider = aggregated.results[0]
        .breakdown
        .providers
        .get("unknown")
        .unwrap();
    assert_eq!(provider.metrics.flat_cost, amount(0));
    assert_eq!(provider.metrics.spend, amount(5));
}

#[test]
fn provider_api_key_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        provider: Some("openai"),
        api_key: Some(KEY_ONE),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_PROVIDER_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let key = aggregated.results[0]
        .breakdown
        .providers
        .get("openai")
        .unwrap()
        .api_key_breakdown
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metrics.flat_cost, amount(7));
}

#[test]
fn model_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        model: Some("gpt-4"),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_MODEL, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let model = aggregated.results[0].breakdown.models.get("gpt-4").unwrap();
    assert_eq!(model.metrics.flat_cost, amount(7));
}

#[test]
fn model_group_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        model_group: Some("chat"),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_MODEL_GROUP, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let group = aggregated.results[0]
        .breakdown
        .model_groups
        .get("chat")
        .unwrap();
    assert_eq!(group.metrics.flat_cost, amount(7));
}

#[test]
fn mcp_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        mcp: Some("github__search"),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_MCP, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let tool = aggregated.results[0]
        .breakdown
        .mcp_servers
        .get("github__search")
        .unwrap();
    assert_eq!(tool.metrics.flat_cost, amount(7));
}

#[test]
fn endpoint_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        endpoint: Some("/v1/chat/completions"),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_ENDPOINT, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let endpoint = aggregated.results[0]
        .breakdown
        .endpoints
        .get("/v1/chat/completions")
        .unwrap();
    assert_eq!(endpoint.metrics.flat_cost, amount(7));
}

#[test]
fn api_key_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        api_key: Some(KEY_ONE),
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE_API_KEY, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    let key = aggregated.results[0]
        .breakdown
        .api_keys
        .get(KEY_ONE)
        .unwrap();
    assert_eq!(key.metrics.flat_cost, amount(7));
}

#[test]
fn date_row_keeps_the_flat_cost() {
    let records = [row(RowParts {
        flat_cost: Some(7),
        ..RowParts::new(GROUP_DATE, Some(DAY_ONE))
    })];
    let aggregated = aggregate_grouping_rows_sync(&records, &BTreeMap::new()).unwrap();
    assert_eq!(aggregated.results[0].metrics.flat_cost, amount(7));
}
