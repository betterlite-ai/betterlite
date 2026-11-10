#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::NaiveDate;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::daily_metrics::{
    BreakdownMetrics, DailySpendData, DailySpendMetadata, DailySpendPage, KeyMetadata,
    KeyMetricWithMetadata, MetricWithMetadata, SpendMetrics,
};

const SPEND_ZERO: &str = concat!(
    r#"{"spend":0.0,"flat_cost":0.0,"prompt_tokens":0,"completion_tokens":0,"#,
    r#""cache_read_input_tokens":0,"cache_creation_input_tokens":0,"compression_saved_tokens":0,"#,
    r#""compression_savings_spend":0.0,"prompt_caching_savings_spend":0.0,"#,
    r#""gateway_injected_caching_savings_spend":0.0,"autorouter_savings_spend":0.0,"#,
    r#""total_tokens":0,"successful_requests":0,"failed_requests":0,"api_requests":0,"#,
    r#""total_response_time_ms":0,"timed_requests":0}"#
);

const BREAKDOWN_EMPTY: &str = concat!(
    r#"{"mcp_servers":{},"models":{},"model_groups":{},"providers":{},"#,
    r#""endpoints":{},"api_keys":{},"entities":{}}"#
);

const METADATA_DEFAULT: &str = concat!(
    r#"{"total_spend":0.0,"total_flat_cost":0.0,"total_prompt_tokens":0,"#,
    r#""total_completion_tokens":0,"total_tokens":0,"total_api_requests":0,"#,
    r#""total_successful_requests":0,"total_failed_requests":0,"#,
    r#""total_cache_read_input_tokens":0,"total_cache_creation_input_tokens":0,"#,
    r#""total_compression_saved_tokens":0,"total_compression_savings_spend":0.0,"#,
    r#""total_prompt_caching_savings_spend":0.0,"#,
    r#""total_gateway_injected_caching_savings_spend":0.0,"#,
    r#""total_autorouter_savings_spend":0.0,"total_response_time_ms":0,"#,
    r#""total_timed_requests":0,"page":1,"total_pages":1,"has_more":false,"#,
    r#""api_key_limit":null,"total_api_keys":null,"entity_total_api_keys":null}"#
);

const KEY_METADATA_DEFAULT: &str = concat!(
    r#"{"key_alias":null,"team_id":null,"user_id":null,"user_email":null,"#,
    r#""key_exists":null}"#
);

#[test]
fn spend_metrics_defaults_money_to_zero() {
    let metrics = SpendMetrics::default();
    assert_eq!(metrics.spend, Decimal::ZERO);
    assert_eq!(metrics.flat_cost, Decimal::ZERO);
    assert_eq!(metrics.compression_savings_spend, Decimal::ZERO);
    assert_eq!(metrics.prompt_caching_savings_spend, Decimal::ZERO);
    assert_eq!(metrics.gateway_caching_savings, Decimal::ZERO);
    assert_eq!(metrics.autorouter_savings_spend, Decimal::ZERO);
}

#[test]
fn spend_metrics_defaults_counts_to_zero() {
    let metrics = SpendMetrics::default();
    assert_eq!(metrics.prompt_tokens, 0);
    assert_eq!(metrics.completion_tokens, 0);
    assert_eq!(metrics.cache_read_input_tokens, 0);
    assert_eq!(metrics.cache_creation_input_tokens, 0);
    assert_eq!(metrics.compression_saved_tokens, 0);
    assert_eq!(metrics.total_tokens, 0);
    assert_eq!(metrics.successful_requests, 0);
    assert_eq!(metrics.failed_requests, 0);
    assert_eq!(metrics.api_requests, 0);
    assert_eq!(metrics.total_response_time_ms, 0);
    assert_eq!(metrics.timed_requests, 0);
}

#[test]
fn spend_metrics_serializes_every_defaulted_field() {
    assert_eq!(
        serde_json::to_string(&SpendMetrics::default()).unwrap(),
        SPEND_ZERO
    );
}

#[test]
fn spend_metrics_reads_and_writes_the_litellm_field_names() {
    let source = concat!(
        r#"{"spend":1.25,"flat_cost":0.5,"prompt_tokens":10,"completion_tokens":20,"#,
        r#""cache_read_input_tokens":3,"cache_creation_input_tokens":4,"#,
        r#""compression_saved_tokens":5,"compression_savings_spend":0.1,"#,
        r#""prompt_caching_savings_spend":0.2,"#,
        r#""gateway_injected_caching_savings_spend":0.3,"#,
        r#""autorouter_savings_spend":0.4,"total_tokens":30,"#,
        r#""successful_requests":2,"failed_requests":1,"api_requests":3,"#,
        r#""total_response_time_ms":900,"timed_requests":2}"#
    );
    let metrics: SpendMetrics = serde_json::from_str(source).unwrap();
    assert_eq!(metrics.spend, Decimal::new(125, 2));
    assert_eq!(metrics.flat_cost, Decimal::new(5, 1));
    assert_eq!(metrics.gateway_caching_savings, Decimal::new(3, 1));
    assert_eq!(metrics.prompt_tokens, 10);
    assert_eq!(metrics.total_tokens, 30);
    assert_eq!(metrics.total_response_time_ms, 900);
    assert_eq!(serde_json::to_string(&metrics).unwrap(), source);
}

#[test]
fn spend_metrics_takes_its_defaults_when_fields_are_absent() {
    let metrics: SpendMetrics = serde_json::from_str("{}").unwrap();
    assert_eq!(metrics, SpendMetrics::default());
    let explicit_zero: SpendMetrics = serde_json::from_str(r#"{"spend":0}"#).unwrap();
    assert_eq!(explicit_zero, SpendMetrics::default());
}

#[test]
fn breakdown_metrics_defaults_to_empty_breakdowns() {
    let breakdown = BreakdownMetrics::default();
    assert!(breakdown.mcp_servers.is_empty());
    assert!(breakdown.models.is_empty());
    assert!(breakdown.model_groups.is_empty());
    assert!(breakdown.providers.is_empty());
    assert!(breakdown.endpoints.is_empty());
    assert!(breakdown.api_keys.is_empty());
    assert!(breakdown.entities.is_empty());
    assert_eq!(serde_json::to_string(&breakdown).unwrap(), BREAKDOWN_EMPTY);
    let parsed: BreakdownMetrics = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, BreakdownMetrics::default());
}

#[test]
fn breakdown_metrics_carries_nested_key_breakdown_and_metadata() {
    let source = concat!(
        r#"{"models":{"gpt-4o":{"metrics":{"spend":2.0},"#,
        r#""metadata":{"owned_by":"team-a"},"#,
        r#""api_key_breakdown":{"sk-1":{"metrics":{"spend":1.5},"#,
        r#""metadata":{"key_alias":"first","team_id":null,"user_id":"u","#,
        r#""user_email":null,"key_exists":false}}}}}}"#
    );
    let breakdown: BreakdownMetrics = serde_json::from_str(source).unwrap();
    let model = breakdown.models.get("gpt-4o").unwrap();
    assert_eq!(model.metrics.spend, Decimal::new(2, 0));
    assert_eq!(
        model.metadata.get("owned_by"),
        Some(&Json::Text("team-a".to_owned()))
    );
    let key = model.api_key_breakdown.get("sk-1").unwrap();
    assert_eq!(key.metrics.spend, Decimal::new(15, 1));
    assert_eq!(key.metadata.key_alias.as_deref(), Some("first"));
    assert_eq!(key.metadata.user_id.as_deref(), Some("u"));
    assert_eq!(key.metadata.key_exists, Some(false));
    assert!(breakdown.api_keys.is_empty());
}

#[test]
fn key_metadata_defaults_every_field_to_none() {
    let metadata = KeyMetadata::default();
    assert_eq!(metadata.key_alias, None);
    assert_eq!(metadata.team_id, None);
    assert_eq!(metadata.user_id, None);
    assert_eq!(metadata.user_email, None);
    assert_eq!(metadata.key_exists, None);
    assert_eq!(
        serde_json::to_string(&metadata).unwrap(),
        KEY_METADATA_DEFAULT
    );
    let parsed: KeyMetadata = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, KeyMetadata::default());
}

#[test]
fn key_metric_with_metadata_defaults_its_metadata() {
    let metric: KeyMetricWithMetadata = serde_json::from_str(r#"{"metrics":{}}"#).unwrap();
    assert_eq!(metric.metrics, SpendMetrics::default());
    assert_eq!(metric.metadata, KeyMetadata::default());
    let expected = [
        r#"{"metrics":"#,
        SPEND_ZERO,
        r#","metadata":"#,
        KEY_METADATA_DEFAULT,
        "}",
    ]
    .concat();
    assert_eq!(serde_json::to_string(&metric).unwrap(), expected);
}

#[test]
fn metric_with_metadata_defaults_its_empty_metadata_and_breakdown() {
    let metric: MetricWithMetadata = serde_json::from_str(r#"{"metrics":{}}"#).unwrap();
    assert_eq!(metric.metrics, SpendMetrics::default());
    assert!(metric.metadata.is_empty());
    assert!(metric.api_key_breakdown.is_empty());
    let expected = [
        r#"{"metrics":"#,
        SPEND_ZERO,
        r#","metadata":{},"api_key_breakdown":{}}"#,
    ]
    .concat();
    assert_eq!(serde_json::to_string(&metric).unwrap(), expected);
}

#[test]
fn daily_spend_metadata_defaults_every_total_to_zero() {
    let metadata = DailySpendMetadata::default();
    assert_eq!(metadata.total_spend, Decimal::ZERO);
    assert_eq!(metadata.total_flat_cost, Decimal::ZERO);
    assert_eq!(metadata.total_cache_read_input_tokens, 0);
    assert_eq!(metadata.total_cache_creation_tokens, 0);
    assert_eq!(metadata.total_compression_savings, Decimal::ZERO);
    assert_eq!(metadata.total_prompt_caching_savings, Decimal::ZERO);
    assert_eq!(metadata.total_gateway_caching_savings, Decimal::ZERO);
    assert_eq!(metadata.total_autorouter_savings_spend, Decimal::ZERO);
}

#[test]
fn daily_spend_metadata_defaults_the_page_fields() {
    let metadata = DailySpendMetadata::default();
    assert_eq!(metadata.page, 1);
    assert_eq!(metadata.total_pages, 1);
    assert!(!metadata.has_more);
    assert_eq!(metadata.api_key_limit, None);
    assert_eq!(metadata.total_api_keys, None);
    assert_eq!(metadata.entity_total_api_keys, None);
    assert_eq!(serde_json::to_string(&metadata).unwrap(), METADATA_DEFAULT);
}

#[test]
fn daily_spend_metadata_reads_values_and_defaults_the_rest() {
    let source = concat!(
        r#"{"total_spend":3.5,"total_tokens":12,"page":2,"has_more":true,"#,
        r#""api_key_limit":5,"entity_total_api_keys":{"team-a":3}}"#
    );
    let metadata: DailySpendMetadata = serde_json::from_str(source).unwrap();
    assert_eq!(metadata.total_spend, Decimal::new(35, 1));
    assert_eq!(metadata.total_tokens, 12);
    assert_eq!(metadata.page, 2);
    assert!(metadata.has_more);
    assert_eq!(metadata.api_key_limit, Some(5));
    assert_eq!(
        metadata.entity_total_api_keys,
        Some(IndexMap::from([("team-a".to_owned(), 3)]))
    );
    assert_eq!(metadata.total_flat_cost, Decimal::ZERO);
    assert_eq!(metadata.total_pages, 1);
}

#[test]
fn daily_spend_page_serializes_like_litellm() {
    let page = DailySpendPage {
        results: vec![DailySpendData {
            date: NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
            metrics: SpendMetrics::default(),
            breakdown: BreakdownMetrics::default(),
        }],
        metadata: DailySpendMetadata::default(),
    };
    let expected = [
        r#"{"results":[{"date":"2026-09-24","metrics":"#,
        SPEND_ZERO,
        r#","breakdown":"#,
        BREAKDOWN_EMPTY,
        r#"}],"metadata":"#,
        METADATA_DEFAULT,
        "}",
    ]
    .concat();
    let json = serde_json::to_string(&page).unwrap();
    assert_eq!(json, expected);
    let parsed: DailySpendPage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, page);
}

#[test]
fn daily_spend_page_defaults_metadata_and_requires_results() {
    let page: DailySpendPage = serde_json::from_str(r#"{"results":[]}"#).unwrap();
    assert_eq!(page.metadata, DailySpendMetadata::default());
    serde_json::from_str::<DailySpendPage>(r#"{"metadata":{}}"#).unwrap_err();
}

#[rstest]
#[case::missing_date(r#"{"metrics":{}}"#)]
#[case::missing_metrics(r#"{"date":"2026-09-24"}"#)]
fn daily_spend_data_requires_date_and_metrics(#[case] source: &str) {
    serde_json::from_str::<DailySpendData>(source).unwrap_err();
}

#[test]
fn daily_spend_data_defaults_its_breakdown() {
    let data: DailySpendData =
        serde_json::from_str(r#"{"date":"2026-09-24","metrics":{}}"#).unwrap();
    assert_eq!(data.date, NaiveDate::from_ymd_opt(2026, 9, 24).unwrap());
    assert_eq!(data.breakdown, BreakdownMetrics::default());
}
