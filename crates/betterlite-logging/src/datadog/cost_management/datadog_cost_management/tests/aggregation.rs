use betterlite_models::decisions::Json;
use jiff::tz::{Offset, TimeZone};
use rust_decimal::Decimal;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::aggregate;

fn entries(payloads: &[&support::Raw]) -> Vec<aggregate::FocusCostEntry> {
    let settings = support::settings(&support::init(&[]));
    let logs: Vec<_> = payloads
        .iter()
        .map(|payload| support::payload(&payload.0))
        .collect();
    aggregate::aggregate(&logs, &settings, &TimeZone::UTC)
}

#[test]
fn aggregation_sums_matching_provider_model_date_and_tags() {
    let first = support::default_payload().set("/response_cost", support::number(0.5));
    let second = support::default_payload().set("/response_cost", support::number(0.25));
    let found = entries(&[&first, &second]);
    assert_eq!(found.len(), 1);
    let entry = found.first().expect("one aggregated entry");
    assert_eq!(entry.provider_name, "openai");
    assert_eq!(entry.charge_description, "LLM Usage for gpt-4o");
    assert_eq!(entry.charge_period_start, "2024-01-01");
    assert_eq!(entry.charge_period_end, "2024-01-01");
    assert_eq!(entry.billed_cost, Decimal::new(75, 2));
    assert_eq!(entry.billing_currency, "USD");
    assert_eq!(support::tag(&entry.tags, "model"), Some("gpt-4o"));
}

#[test]
fn aggregation_keeps_providers_models_and_days_apart() {
    let openai = support::default_payload();
    let anthropic =
        support::default_payload().set("/custom_llm_provider", support::text("anthropic"));
    let other_model = support::default_payload().set("/model", support::text("gpt-4o-mini"));
    let other_day = support::default_payload().set("/startTime", support::number(1_704_196_800.0));
    let found = entries(&[&openai, &anthropic, &other_model, &other_day]);
    assert_eq!(found.len(), 4);
    assert_eq!(
        found
            .first()
            .map(|entry| entry.charge_period_start.as_str()),
        Some("2024-01-01")
    );
    assert_eq!(
        found.last().map(|entry| entry.charge_period_start.as_str()),
        Some("2024-01-02")
    );
}

#[test]
fn aggregation_keeps_different_tags_apart() {
    let alice =
        support::default_payload().set("/metadata/user_api_key_alias", support::text("alice"));
    let bob = support::default_payload().set("/metadata/user_api_key_alias", support::text("bob"));
    let found = entries(&[&alice, &bob]);
    assert_eq!(found.len(), 2);
    assert_eq!(
        found
            .first()
            .and_then(|entry| support::tag(&entry.tags, "user")),
        Some("alice")
    );
    assert_eq!(
        found
            .last()
            .and_then(|entry| support::tag(&entry.tags, "user")),
        Some("bob")
    );
}

#[test]
fn aggregation_names_a_missing_provider_unknown() {
    let payload = support::default_payload().set("/custom_llm_provider", Json::Null);
    let found = entries(&[&payload]);
    let entry = found.first().expect("one entry");
    assert_eq!(entry.provider_name, "unknown");
    assert_eq!(support::tag(&entry.tags, "provider"), None);
}

#[test]
fn aggregation_skips_an_undatable_record() {
    let late = support::default_payload().set("/startTime", support::number(9.3e18));
    let found = entries(&[&late]);
    assert!(found.is_empty());
}

#[test]
fn aggregation_formats_the_period_in_the_injected_timezone() {
    let late = support::default_payload().set("/startTime", support::number(1_704_065_400.0));
    let settings = support::settings(&support::init(&[]));
    let logs = [support::payload(&late.0)];
    let utc = aggregate::aggregate(&logs, &settings, &TimeZone::UTC);
    let offset = Offset::from_hours(2).expect("a valid offset");
    let local = aggregate::aggregate(&logs, &settings, &TimeZone::fixed(offset));
    assert_eq!(
        utc.first().map(|entry| entry.charge_period_start.as_str()),
        Some("2023-12-31")
    );
    assert_eq!(
        local
            .first()
            .map(|entry| entry.charge_period_start.as_str()),
        Some("2024-01-01")
    );
}
