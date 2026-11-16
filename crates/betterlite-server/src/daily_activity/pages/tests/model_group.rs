#![cfg(test)]

use betterlite_db::daily_activity_sql::PTU_SENTINEL_API_KEY;

use super::record::DailySpendRecord;
use super::support::{BreakdownFixture, breakdown_of, money, spend_record, text};

fn record_with(model: Option<&str>, model_group: Option<&str>, api_key: &str) -> DailySpendRecord {
    let mut record = spend_record("2025-03-10", api_key);
    record.model = model.map(text);
    record.model_group = model_group.map(text);
    record.spend = money("1.25");
    record
}

#[test]
fn real_keys_bucket_models_by_model_only() {
    let record = record_with(Some("gpt-4o"), Some("group-a"), "sk-real");
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert_eq!(breakdown.models.len(), 1);
    assert!(breakdown.models.contains_key("gpt-4o"));
    assert_eq!(
        breakdown.models["gpt-4o"].api_key_breakdown["sk-real"]
            .metrics
            .spend,
        money("1.25")
    );
    assert_eq!(breakdown.model_groups.len(), 1);
    assert_eq!(
        breakdown.model_groups["group-a"].metrics.spend,
        money("1.25")
    );
    assert_eq!(
        breakdown.model_groups["group-a"].api_key_breakdown["sk-real"]
            .metrics
            .spend,
        money("1.25")
    );
}

#[test]
fn sentinels_bucket_models_by_the_group_name() {
    let record = record_with(
        Some("deployment-1"),
        Some("display-name"),
        PTU_SENTINEL_API_KEY,
    );
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert_eq!(breakdown.models.len(), 1);
    assert!(breakdown.models.contains_key("display-name"));
    assert!(!breakdown.models.contains_key("deployment-1"));
    assert_eq!(
        breakdown.models["display-name"].metrics.spend,
        money("1.25")
    );
}

#[test]
fn sentinels_fall_back_to_the_model_without_a_group() {
    let record = record_with(Some("gpt-4o"), None, PTU_SENTINEL_API_KEY);
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert!(breakdown.models.contains_key("gpt-4o"));
    assert!(breakdown.model_groups.contains_key("gpt-4o"));
}

#[test]
fn model_groups_fall_back_to_the_model_for_real_keys() {
    let record = record_with(Some("gpt-4o"), None, "sk-real");
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert!(breakdown.model_groups.contains_key("gpt-4o"));
    assert_eq!(
        breakdown.model_groups["gpt-4o"].metrics.spend,
        money("1.25")
    );
}

#[test]
fn a_real_key_without_a_model_skips_the_models_bucket() {
    let record = record_with(None, Some("group-a"), "sk-real");
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert!(breakdown.models.is_empty());
    assert!(breakdown.model_groups.contains_key("group-a"));
    assert_eq!(breakdown.api_keys["sk-real"].metrics.spend, money("1.25"));
}

#[test]
fn a_sentinel_without_any_model_skips_both_buckets() {
    let record = record_with(None, None, PTU_SENTINEL_API_KEY);
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert!(breakdown.models.is_empty());
    assert!(breakdown.model_groups.is_empty());
}

#[test]
fn an_empty_model_name_is_bucketed_as_given() {
    let record = record_with(Some(""), Some("group-a"), "sk-real");
    let breakdown = breakdown_of(&[record], &BreakdownFixture::default()).unwrap();
    assert!(breakdown.models.contains_key(""));
    assert_eq!(breakdown.models[""].metrics.spend, money("1.25"));
}
