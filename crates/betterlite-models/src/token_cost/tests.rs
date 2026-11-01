#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::*;
use crate::cost_usage::CostUsage;
use crate::price_lookup::{PriceLookup, PriceRecord};

fn fixture_usage(prompt_tokens: i64, completion_tokens: i64) -> CostUsage {
    CostUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: 0,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn model(entries: &[(&str, &str)]) -> ModelInfo {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
}

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

struct CountingLookup {
    answer: Option<PriceRecord>,
    calls: Cell<u32>,
}

impl PriceLookup for CountingLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.answer.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

fn base_request<'fixture>(
    usage: &'fixture CostUsage,
    model_info: &'fixture ModelInfo,
) -> GenericCostRequest<'fixture> {
    GenericCostRequest {
        model: "fixture-model",
        custom_llm_provider: None,
        usage,
        prompt_extras: None,
        supplied_model_info: Some(model_info),
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        moment: moment(),
        tiered_pricing: &[],
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    }
}

#[test]
fn an_unregistered_model_without_supplied_pricing_errors() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(10, 5);
    let model_info = ModelInfo::new();
    let mut request = base_request(&usage, &model_info);
    request.supplied_model_info = None;
    let result = generic_cost_per_token(&request, &NullLookup, &database);
    assert!(matches!(result, Err(TokenCostError::UnknownModel)));
}

#[test]
fn a_supplied_empty_model_record_bypasses_the_catalog_lookup() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(10, 5);
    let model_info = ModelInfo::new();
    let request = base_request(&usage, &model_info);
    let lookup = CountingLookup {
        answer: Some(model(&[("input_cost_per_token", "\"1.0\"")])),
        calls: Cell::new(0),
    };
    let result = generic_cost_per_token(&request, &lookup, &database)
        .expect("an empty record is still usable");
    assert_eq!(lookup.calls.get(), 0, "the catalog lookup must never run");
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn exact_totals_apply_the_residency_then_vertex_uplift_to_both_sides() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(100, 50);
    let model_info = model(&[
        ("input_cost_per_token", "\"0.001\""),
        ("output_cost_per_token", "\"0.002\""),
        ("regional_processing_uplift_multiplier_us", "\"1.1\""),
        ("regional_endpoint_uplift_multiplier", "\"1.05\""),
    ]);
    let mut request = base_request(&usage, &model_info);
    request.data_residency = Some("us");
    request.vertex_location = Some("us-central1");

    let result = generic_cost_per_token(&request, &NullLookup, &database)
        .expect("exact arithmetic succeeds");
    assert_eq!(
        result.prompt_cost,
        Decimal::from_str_exact("0.1155").expect("literal")
    );
    assert_eq!(
        result.completion_cost,
        Decimal::from_str_exact("0.1155").expect("literal")
    );
}

#[test]
fn an_unrecognized_residency_leaves_totals_unmultiplied() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(10, 10);
    let model_info = model(&[
        ("input_cost_per_token", "\"0.01\""),
        ("output_cost_per_token", "\"0.01\""),
        ("regional_processing_uplift_multiplier_us", "\"2.0\""),
    ]);
    let mut request = base_request(&usage, &model_info);
    request.data_residency = Some("ca");

    let result = generic_cost_per_token(&request, &NullLookup, &database)
        .expect("exact arithmetic succeeds");
    assert_eq!(
        result.prompt_cost,
        Decimal::from_str_exact("0.1").expect("literal")
    );
    assert_eq!(
        result.completion_cost,
        Decimal::from_str_exact("0.1").expect("literal")
    );
}
