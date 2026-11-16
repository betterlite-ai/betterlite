#![cfg(test)]

use betterlite_analytics::daily_metrics::SpendMetrics;
use rust_decimal::Decimal;

use super::read::{PageCallbacks, get_daily_activity};
use super::record::DailySpendRecord;
use super::support::{metrics, money, page_repository, request, row};

#[tokio::test]
async fn callback_overrides_the_computed_totals() {
    let callback = |_records: &[DailySpendRecord]| -> SpendMetrics { metrics(money("42"), 7, 0) };
    let mut record = row("1", "2025-03-10", "sk-a");
    record.spend = money("1");
    let repository = page_repository(vec![record], 1);
    let mut request = request("2025-03-01", "2025-03-31");
    request.callbacks = PageCallbacks {
        metadata_metrics: Some(&callback),
        resolve_entity_metadata: None,
    };
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.total_spend, money("42"));
    assert_eq!(response.metadata.total_tokens, 7);
    assert_eq!(response.results[0].metrics.spend, money("1"));
}

#[tokio::test]
async fn absent_callback_uses_the_aggregated_totals() {
    let mut record = row("1", "2025-03-10", "sk-a");
    record.spend = money("3");
    let repository = page_repository(vec![record], 1);
    let request = request("2025-03-01", "2025-03-31");
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.total_spend, money("3"));
    assert_eq!(response.metadata.total_tokens, 0);
}

#[tokio::test]
async fn callback_receives_the_page_records() {
    let callback = |records: &[DailySpendRecord]| -> SpendMetrics {
        let spend: Decimal = records.iter().map(|record| record.spend).sum();
        let requests = i64::try_from(records.len()).unwrap();
        metrics(spend, 0, requests)
    };
    let mut first = row("1", "2025-03-10", "sk-a");
    first.spend = money("1");
    let mut second = row("2", "2025-03-11", "sk-a");
    second.spend = money("2");
    let repository = page_repository(vec![first, second], 20);
    let mut request = request("2025-03-01", "2025-03-31");
    request.callbacks = PageCallbacks {
        metadata_metrics: Some(&callback),
        resolve_entity_metadata: None,
    };
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.total_spend, money("3"));
    assert_eq!(response.metadata.total_api_requests, 2);
    assert_eq!(response.metadata.total_pages, 2);
}
