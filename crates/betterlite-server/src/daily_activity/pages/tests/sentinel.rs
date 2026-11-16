#![cfg(test)]

use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;

use super::aggregate::aggregate_spend_records;
use super::record::DailySpendRecord;
use super::support::{BreakdownFixture, breakdown_of, money, page_repository, spend_record, text};

fn sentinel_record() -> DailySpendRecord {
    let mut record = spend_record("2025-03-10", PTU_SENTINEL_API_KEY);
    record.model = Some(text("gpt-4o"));
    record.model_group = Some(text("group-a"));
    record.custom_llm_provider = Some(text("openai"));
    record.mcp_namespaced_tool_name = Some(text("mcp-1"));
    record.endpoint = Some(text("/v1/chat/completions"));
    record.entity_id = Some(text("user-7"));
    record.spend = money("1.25");
    record.flat_cost = money("0.5");
    record
}

fn fixture() -> BreakdownFixture {
    BreakdownFixture::default().with_entity_field("user_id")
}

#[test]
fn sentinels_are_kept_out_of_models_providers_and_api_keys() {
    let breakdown = breakdown_of(&[sentinel_record()], &fixture()).unwrap();
    assert!(breakdown.providers.is_empty());
    assert!(breakdown.api_keys.is_empty());
    assert_eq!(breakdown.models.len(), 1);
    assert!(!breakdown.models.contains_key("gpt-4o"));
    assert!(breakdown.models["group-a"].api_key_breakdown.is_empty());
    assert!(
        breakdown.model_groups["group-a"]
            .api_key_breakdown
            .is_empty()
    );
    assert!(breakdown.entities["user-7"].api_key_breakdown.is_empty());
}

#[test]
fn sentinels_add_flat_cost_to_their_parent_buckets() {
    let breakdown = breakdown_of(&[sentinel_record()], &fixture()).unwrap();
    let model = &breakdown.models["group-a"].metrics;
    assert_eq!(model.spend, money("1.25"));
    assert_eq!(model.flat_cost, money("0.5"));
    assert_eq!(
        breakdown.model_groups["group-a"].metrics.flat_cost,
        money("0.5")
    );
    assert_eq!(breakdown.entities["user-7"].metrics.flat_cost, money("0.5"));
}

#[test]
fn sentinels_keep_key_rows_on_mcp_and_endpoint_buckets() {
    let breakdown = breakdown_of(&[sentinel_record()], &fixture()).unwrap();
    let mcp = &breakdown.mcp_servers["mcp-1"];
    let endpoint = &breakdown.endpoints["/v1/chat/completions"];
    assert_eq!(mcp.metrics.flat_cost, money("0.5"));
    assert_eq!(endpoint.metrics.flat_cost, money("0.5"));
    let mcp_key = &mcp.api_key_breakdown[PTU_SENTINEL_API_KEY];
    assert_eq!(mcp_key.metrics.flat_cost, money("0.5"));
    assert_eq!(mcp_key.metadata.key_exists, Some(false));
    assert_eq!(
        endpoint.api_key_breakdown[PTU_SENTINEL_API_KEY]
            .metrics
            .spend,
        money("1.25")
    );
}

#[test]
fn sentinels_without_flat_cost_still_add_spend() {
    let mut record = sentinel_record();
    record.flat_cost = money("0");
    let breakdown = breakdown_of(&[record], &fixture()).unwrap();
    let model = &breakdown.models["group-a"].metrics;
    assert_eq!(model.spend, money("1.25"));
    assert_eq!(model.flat_cost, money("0"));
}

#[tokio::test]
async fn sentinels_alone_do_not_fetch_key_metadata() {
    let repository = page_repository(Vec::new(), 0);
    let aggregated =
        aggregate_spend_records(&repository, &[sentinel_record()], Some("user_id"), None)
            .await
            .unwrap();
    assert!(repository.proxy_reads.read_calls().is_empty());
    assert!(repository.database.token_calls().is_empty());
    assert_eq!(aggregated.results[0].metrics.flat_cost, money("0.5"));
    assert!(aggregated.results[0].breakdown.api_keys.is_empty());
}
