#![cfg(test)]

use betterlite_analytics::daily_metrics::SpendMetrics;
use rust_decimal::Decimal;

use super::super::error::PagesError;
use super::super::record::DailySpendRecord;
use super::super::support::{
    BreakdownFixture, breakdown_of, entity_metadata, fields, key_row, money, spend_record, text,
};

fn distinct_record() -> DailySpendRecord {
    let mut record = spend_record("2025-03-10", "sk-real");
    record.model = Some(text("gpt-4o"));
    record.spend = money("1.25");
    record.flat_cost = money("0.5");
    record.prompt_tokens = 100;
    record.completion_tokens = 40;
    record.cache_read_input_tokens = 7;
    record.cache_creation_input_tokens = 11;
    record.compression_saved_tokens = 3;
    record.compression_savings_spend = money("0.2");
    record.prompt_caching_savings_spend = money("0.3");
    record.gateway_caching_savings = money("0.4");
    record.autorouter_savings_spend = money("0.6");
    record.api_requests = 9;
    record.successful_requests = 8;
    record.failed_requests = 1;
    record.total_response_time_ms = 1500;
    record.timed_requests = 6;
    record
}

fn distinct_metrics() -> SpendMetrics {
    SpendMetrics {
        spend: money("1.25"),
        flat_cost: money("0.5"),
        prompt_tokens: 100,
        completion_tokens: 40,
        cache_read_input_tokens: 7,
        cache_creation_input_tokens: 11,
        compression_saved_tokens: 3,
        compression_savings_spend: money("0.2"),
        prompt_caching_savings_spend: money("0.3"),
        gateway_caching_savings: money("0.4"),
        autorouter_savings_spend: money("0.6"),
        total_tokens: 140,
        successful_requests: 8,
        failed_requests: 1,
        api_requests: 9,
        total_response_time_ms: 1500,
        timed_requests: 6,
    }
}

#[test]
fn metrics_carry_every_field_of_the_record() {
    let fixture = BreakdownFixture::default();
    let breakdown = breakdown_of(&[distinct_record()], &fixture).unwrap();
    assert_eq!(breakdown.models["gpt-4o"].metrics, distinct_metrics());
    assert_eq!(breakdown.api_keys["sk-real"].metrics, distinct_metrics());
}

#[test]
fn records_accumulate_into_the_same_bucket() {
    let fixture = BreakdownFixture::default();
    let mut first = spend_record("2025-03-10", "sk-real");
    first.model = Some(text("gpt-4o"));
    first.spend = money("1.25");
    first.prompt_tokens = 10;
    first.completion_tokens = 4;
    let mut second = spend_record("2025-03-11", "sk-real");
    second.model = Some(text("gpt-4o"));
    second.spend = money("0.75");
    second.prompt_tokens = 5;
    second.completion_tokens = 2;
    let breakdown = breakdown_of(&[first, second], &fixture).unwrap();
    let metrics = &breakdown.models["gpt-4o"].metrics;
    assert_eq!(metrics.spend, money("2.00"));
    assert_eq!(metrics.prompt_tokens, 15);
    assert_eq!(metrics.completion_tokens, 6);
    assert_eq!(metrics.total_tokens, 21);
}

#[test]
fn record_reaches_the_model_and_group_buckets() {
    let mut fixture = BreakdownFixture::default();
    fixture
        .model_metadata
        .insert(text("gpt-4o"), fields(&[("display", "GPT-4o")]));
    fixture
        .model_metadata
        .insert(text("group-a"), fields(&[("tier", "gold")]));
    let mut record = spend_record("2025-03-10", "sk-real");
    record.model = Some(text("gpt-4o"));
    record.model_group = Some(text("group-a"));
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert_eq!(breakdown.models["gpt-4o"].metrics.spend, money("1.25"));
    assert_eq!(
        breakdown.models["gpt-4o"].metadata,
        fields(&[("display", "GPT-4o")])
    );
    assert_eq!(
        breakdown.model_groups["group-a"].metrics.spend,
        money("1.25")
    );
    assert_eq!(
        breakdown.model_groups["group-a"].metadata,
        fields(&[("tier", "gold")])
    );
}

#[test]
fn record_reaches_the_mcp_server_and_endpoint_buckets() {
    let fixture = BreakdownFixture::default();
    let mut record = spend_record("2025-03-10", "sk-real");
    record.mcp_namespaced_tool_name = Some(text("mcp-1"));
    record.endpoint = Some(text("/v1/chat/completions"));
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    let mcp = &breakdown.mcp_servers["mcp-1"];
    let endpoint = &breakdown.endpoints["/v1/chat/completions"];
    assert_eq!(mcp.metrics.spend, money("1.25"));
    assert_eq!(endpoint.metrics.spend, money("1.25"));
    assert!(mcp.metadata.is_empty());
    assert!(endpoint.metadata.is_empty());
    assert_eq!(
        mcp.api_key_breakdown["sk-real"].metrics.spend,
        money("1.25")
    );
    assert_eq!(
        endpoint.api_key_breakdown["sk-real"].metrics.spend,
        money("1.25")
    );
}

#[test]
fn record_reaches_the_provider_and_top_level_key_buckets() {
    let mut fixture = BreakdownFixture::default();
    fixture
        .provider_metadata
        .insert(text("openai"), fields(&[("region", "us")]));
    fixture = fixture.with_key_row(key_row("sk-real", "alias-1"));
    let mut record = spend_record("2025-03-10", "sk-real");
    record.custom_llm_provider = Some(text("openai"));
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert_eq!(breakdown.providers["openai"].metrics.spend, money("1.25"));
    assert_eq!(
        breakdown.providers["openai"].metadata,
        fields(&[("region", "us")])
    );
    let key = &breakdown.api_keys["sk-real"];
    assert_eq!(key.metrics.spend, money("1.25"));
    assert_eq!(key.metadata.key_alias.as_deref(), Some("alias-1"));
    assert_eq!(key.metadata.team_id.as_deref(), Some("team-1"));
    assert_eq!(key.metadata.user_id.as_deref(), Some("user-1"));
    assert_eq!(key.metadata.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(key.metadata.key_exists, Some(true));
}

#[test]
fn unknown_api_keys_are_marked_missing() {
    let fixture = BreakdownFixture::default();
    let record = spend_record("2025-03-10", "sk-gone");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    let metadata = &breakdown.api_keys["sk-gone"].metadata;
    assert_eq!(metadata.key_alias, None);
    assert_eq!(metadata.team_id, None);
    assert_eq!(metadata.user_id, None);
    assert_eq!(metadata.user_email, None);
    assert_eq!(metadata.key_exists, Some(false));
}

#[test]
fn missing_values_leave_the_dimensions_empty() {
    let fixture = BreakdownFixture::default();
    let record = spend_record("2025-03-10", "sk-real");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert!(breakdown.models.is_empty());
    assert!(breakdown.model_groups.is_empty());
    assert!(breakdown.mcp_servers.is_empty());
    assert!(breakdown.endpoints.is_empty());
    assert!(breakdown.providers.is_empty());
    assert!(breakdown.entities.is_empty());
    assert_eq!(breakdown.api_keys.len(), 1);
}

#[test]
fn absent_providers_are_keyed_as_unknown() {
    let fixture = BreakdownFixture::default();
    let mut record = spend_record("2025-03-10", "sk-real");
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert_eq!(breakdown.providers.len(), 1);
    assert_eq!(breakdown.providers["unknown"].metrics.spend, money("1.25"));
    assert_eq!(
        breakdown.providers["unknown"].api_key_breakdown["sk-real"]
            .metrics
            .spend,
        money("1.25")
    );
}

#[test]
fn entities_use_the_record_id_and_its_metadata() {
    let fixture = BreakdownFixture::default()
        .with_entity_field("user_id")
        .with_entity_metadata(entity_metadata(&[
            ("user-7", fields(&[("plan", "pro")])),
            ("Unassigned", fields(&[("plan", "free")])),
        ]));
    let mut record = spend_record("2025-03-10", "sk-real");
    record.entity_id = Some(text("user-7"));
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert_eq!(breakdown.entities.len(), 1);
    let entity = &breakdown.entities["user-7"];
    assert_eq!(entity.metrics.spend, money("1.25"));
    assert_eq!(entity.metadata, fields(&[("plan", "pro")]));
    assert_eq!(
        entity.api_key_breakdown["sk-real"].metrics.spend,
        money("1.25")
    );
}

#[test]
fn entities_without_an_id_use_unassigned() {
    let fixture = BreakdownFixture::default()
        .with_entity_field("user_id")
        .with_entity_metadata(entity_metadata(&[(
            "Unassigned",
            fields(&[("plan", "free")]),
        )]));
    let mut record = spend_record("2025-03-10", "sk-real");
    record.spend = money("1.25");
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    let entity = &breakdown.entities["Unassigned"];
    assert_eq!(entity.metrics.spend, money("1.25"));
    assert_eq!(entity.metadata, fields(&[("plan", "free")]));
}

#[test]
fn no_entity_field_means_no_entity_bucket() {
    let fixture = BreakdownFixture::default();
    let mut record = spend_record("2025-03-10", "sk-real");
    record.entity_id = Some(text("user-7"));
    let breakdown = breakdown_of(&[record], &fixture).unwrap();
    assert!(breakdown.entities.is_empty());
}

#[test]
fn decimal_overflow_is_reported() {
    let fixture = BreakdownFixture::default();
    let mut huge = spend_record("2025-03-10", "sk-real");
    huge.model = Some(text("gpt-4o"));
    huge.spend = Decimal::MAX;
    let mut extra = spend_record("2025-03-10", "sk-real");
    extra.model = Some(text("gpt-4o"));
    extra.spend = money("1");
    let error = breakdown_of(&[huge, extra], &fixture).unwrap_err();
    assert!(matches!(error, PagesError::Overflow));
}

#[test]
fn token_overflow_is_reported() {
    let fixture = BreakdownFixture::default();
    let mut record = spend_record("2025-03-10", "sk-real");
    record.model = Some(text("gpt-4o"));
    record.prompt_tokens = i64::MAX;
    record.completion_tokens = 1;
    let error = breakdown_of(&[record], &fixture).unwrap_err();
    assert!(matches!(error, PagesError::Overflow));
}
