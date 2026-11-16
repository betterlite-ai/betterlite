#![cfg(test)]

use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{DailyActivityRow, SqlParameter};
use rstest::rstest;
use std::future::ready;

use crate::daily_activity::recovery::scope::ScopeIds;

use super::error::PagesError;
use super::read::{EntityMetadataFuture, PageCallbacks, get_daily_activity};
use super::record::DailySpendRecord;
use super::support::{
    day, entity_metadata, failing_repository, fields, money, page_repository, request, row,
    strings, text,
};

fn distinct_row() -> DailyActivityRow {
    let mut row = row("1", "2025-03-10", "sk-a");
    row.spend = money("1.25");
    row.prompt_tokens = 100;
    row.completion_tokens = 40;
    row.cache_read_input_tokens = 7;
    row.cache_creation_input_tokens = 11;
    row.compression_saved_tokens = 3;
    row.compression_savings_spend = money("0.2");
    row.prompt_caching_savings_spend = money("0.3");
    row.gateway_cache_savings_spend = money("0.4");
    row.autorouter_savings_spend = money("0.6");
    row.api_requests = 9;
    row.successful_requests = 8;
    row.failed_requests = 1;
    row.total_response_time_ms = 1500;
    row.timed_requests = 6;
    row
}

#[tokio::test]
async fn parses_dates_and_queries_the_requested_page() {
    let repository = page_repository(Vec::new(), 0);
    let mut request = request("2025-03-01", "2025-03-31");
    request.page = 2;
    request.page_size = 5;
    let response = get_daily_activity(&repository, &request).await.unwrap();
    let calls = repository.database.find_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].table, DailyActivityTable::User);
    assert_eq!(calls[0].skip, 5);
    assert_eq!(calls[0].take, 5);
    assert_eq!(
        calls[0].predicate.params[0],
        SqlParameter::Text(text("2025-03-01"))
    );
    assert_eq!(
        calls[0].predicate.params[1],
        SqlParameter::Text(text("2025-03-31"))
    );
    assert_eq!(
        repository.database.count_calls(),
        vec![DailyActivityTable::User]
    );
    assert_eq!(response.metadata.page, 2);
}

#[tokio::test]
async fn missing_dates_are_refused_before_querying() {
    let repository = page_repository(Vec::new(), 0);
    let mut request = request("2025-03-01", "2025-03-31");
    request.start_date = None;
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(result, Err(PagesError::DateRange(_))));
    assert!(repository.database.find_calls().is_empty());
    assert!(repository.database.count_calls().is_empty());
}

#[rstest]
#[case::impossible_day("2025-02-30", "2025-03-31")]
#[case::unpadded_month("2025-3-01", "2025-03-31")]
#[case::zero_year("0000-03-01", "2025-03-31")]
#[tokio::test]
async fn invalid_dates_are_refused(#[case] start: &str, #[case] end: &str) {
    let repository = page_repository(Vec::new(), 0);
    let request = request(start, end);
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(result, Err(PagesError::DateRange(_))));
    assert!(repository.database.find_calls().is_empty());
}

#[tokio::test]
async fn scope_errors_are_refused_before_querying() {
    let repository = page_repository(Vec::new(), 0);
    let mut request = request("2025-03-01", "2025-03-31");
    request.table = DailyActivityTable::Team;
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(result, Err(PagesError::Scope(_))));
    assert!(repository.database.find_calls().is_empty());
}

#[tokio::test]
async fn api_keys_and_models_reach_the_scope_predicate() {
    let repository = page_repository(Vec::new(), 0);
    let mut request = request("2025-03-01", "2025-03-31");
    request.api_key = Some(ScopeIds::One(text("sk-team")));
    request.model = Some("gpt-4o");
    get_daily_activity(&repository, &request).await.unwrap();
    let calls = repository.database.find_calls();
    let params = &calls[0].predicate.params;
    assert!(params.contains(&SqlParameter::Text(text("gpt-4o"))));
    assert!(params.contains(&SqlParameter::Texts(strings(&["sk-team"]))));
}

#[tokio::test]
async fn rows_become_aggregated_daily_results() {
    let mut first = row("1", "2025-03-10", "sk-a");
    first.model = Some(text("gpt-4o"));
    first.spend = money("1.25");
    first.prompt_tokens = 10;
    let mut second = row("2", "2025-03-10", "sk-a");
    second.model = Some(text("gpt-4o"));
    second.spend = money("0.75");
    second.prompt_tokens = 5;
    let mut third = row("3", "2025-03-11", "sk-a");
    third.model = Some(text("gpt-4o"));
    third.spend = money("2");
    let repository = page_repository(vec![first, second, third], 3);
    let request = request("2025-03-01", "2025-03-31");
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].date, day(2025, 3, 11));
    assert_eq!(response.results[1].metrics.spend, money("2.00"));
    assert_eq!(response.results[1].metrics.total_tokens, 15);
    assert_eq!(
        response.results[1].breakdown.models["gpt-4o"].metrics.spend,
        money("2.00")
    );
    assert_eq!(response.metadata.total_spend, money("4.00"));
}

#[tokio::test]
async fn page_totals_cover_only_the_returned_rows() {
    let mut first = row("1", "2025-03-10", "sk-a");
    first.spend = money("1");
    let mut second = row("2", "2025-03-11", "sk-a");
    second.spend = money("1");
    let repository = page_repository(vec![first, second], 50);
    let request = request("2025-03-01", "2025-03-31");
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.total_spend, money("2"));
    assert_eq!(response.metadata.total_pages, 5);
    assert_eq!(response.metadata.page, 1);
    assert!(response.metadata.has_more);
}

#[tokio::test]
async fn metadata_totals_follow_the_page_records() {
    let repository = page_repository(vec![distinct_row()], 1);
    let request = request("2025-03-01", "2025-03-31");
    let response = get_daily_activity(&repository, &request).await.unwrap();
    let metadata = &response.metadata;
    assert_eq!(metadata.total_spend, money("1.25"));
    assert_eq!(metadata.total_flat_cost, money("0"));
    assert_eq!(metadata.total_prompt_tokens, 100);
    assert_eq!(metadata.total_completion_tokens, 40);
    assert_eq!(metadata.total_tokens, 140);
    assert_eq!(metadata.total_api_requests, 9);
    assert_eq!(metadata.total_successful_requests, 8);
    assert_eq!(metadata.total_failed_requests, 1);
}

#[tokio::test]
async fn metadata_savings_follow_the_page_records() {
    let repository = page_repository(vec![distinct_row()], 1);
    let request = request("2025-03-01", "2025-03-31");
    let response = get_daily_activity(&repository, &request).await.unwrap();
    let metadata = &response.metadata;
    assert_eq!(metadata.total_cache_read_input_tokens, 7);
    assert_eq!(metadata.total_cache_creation_tokens, 11);
    assert_eq!(metadata.total_compression_saved_tokens, 3);
    assert_eq!(metadata.total_compression_savings, money("0.2"));
    assert_eq!(metadata.total_prompt_caching_savings, money("0.3"));
    assert_eq!(metadata.total_gateway_caching_savings, money("0.4"));
    assert_eq!(metadata.total_autorouter_savings_spend, money("0.6"));
    assert_eq!(metadata.total_response_time_ms, 1500);
    assert_eq!(metadata.total_timed_requests, 6);
    assert_eq!(metadata.api_key_limit, None);
    assert_eq!(metadata.total_api_keys, None);
    assert_eq!(metadata.entity_total_api_keys, None);
}

#[tokio::test]
async fn repository_errors_propagate() {
    let repository = failing_repository();
    let request = request("2025-03-01", "2025-03-31");
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(result, Err(PagesError::Repository(_))));
}

#[tokio::test]
async fn supplied_entity_metadata_reaches_the_results() {
    let supplied = entity_metadata(&[("Unassigned", fields(&[("plan", "free")]))]);
    let repository = page_repository(vec![row("1", "2025-03-10", "sk-a")], 1);
    let mut request = request("2025-03-01", "2025-03-31");
    request.entity_metadata = Some(&supplied);
    let response = get_daily_activity(&repository, &request).await.unwrap();
    let entities = &response.results[0].breakdown.entities;
    assert_eq!(entities["Unassigned"].metadata, fields(&[("plan", "free")]));
}

#[tokio::test]
async fn resolved_entity_metadata_overrides_the_supplied_map() {
    let supplied = entity_metadata(&[("Unassigned", fields(&[("plan", "free")]))]);
    let resolver = |records: &[DailySpendRecord]| -> EntityMetadataFuture {
        let plan = if records.len() == 1 { "one" } else { "many" };
        Box::pin(ready(entity_metadata(&[(
            "Unassigned",
            fields(&[("plan", plan)]),
        )])))
    };
    let repository = page_repository(vec![row("1", "2025-03-10", "sk-a")], 1);
    let mut request = request("2025-03-01", "2025-03-31");
    request.entity_metadata = Some(&supplied);
    request.callbacks = PageCallbacks {
        metadata_metrics: None,
        resolve_entity_metadata: Some(&resolver),
    };
    let response = get_daily_activity(&repository, &request).await.unwrap();
    let entities = &response.results[0].breakdown.entities;
    assert_eq!(entities["Unassigned"].metadata, fields(&[("plan", "one")]));
}
