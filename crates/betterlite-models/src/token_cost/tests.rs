#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::*;
use crate::cost_usage::CostUsage;
use crate::off_peak::schema::OffPeakBlock;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Tier;

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

fn priced(entries: &[(&str, &str)]) -> PricedModel {
    PricedModel {
        record: model(entries),
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    }
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

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

struct CountingLookup {
    answer: Option<PricedModel>,
    calls: Cell<u32>,
}

impl CountingLookup {
    fn new(answer: Option<PricedModel>) -> Self {
        Self {
            answer,
            calls: Cell::new(0),
        }
    }
}

impl PriceLookup for CountingLookup {
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

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.answer.clone()
    }
}

fn base_request<'fixture>(
    usage: &'fixture CostUsage,
    pricing: &'fixture PricedModel,
    prompt_extras: &'fixture PromptExtras,
) -> GenericCostRequest<'fixture> {
    GenericCostRequest {
        model: "fixture-model",
        custom_llm_provider: None,
        usage,
        prompt_extras,
        supplied_pricing: Some(pricing),
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        moment: moment(),
    }
}

#[test]
fn an_unregistered_model_without_supplied_pricing_errors() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(10, 5);
    let pricing = priced(&[]);
    let extras = PromptExtras::default();
    let mut request = base_request(&usage, &pricing, &extras);
    request.supplied_pricing = None;
    let result = generic_cost_per_token(&request, &NullLookup, &database);
    assert!(matches!(result, Err(TokenCostError::UnknownModel)));
}

#[test]
fn a_supplied_empty_model_record_bypasses_the_catalog_lookup() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(10, 5);
    let pricing = priced(&[]);
    let extras = PromptExtras::default();
    let request = base_request(&usage, &pricing, &extras);
    let lookup = CountingLookup::new(Some(priced(&[("input_cost_per_token", "\"1.0\"")])));
    let result = generic_cost_per_token(&request, &lookup, &database)
        .expect("an empty record is still usable");
    assert_eq!(lookup.calls.get(), 0, "the catalog lookup must never run");
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn a_catalog_resolved_model_bills_its_tier_and_off_peak_rates() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(500, 100);
    let extras = PromptExtras::default();
    let unused_pricing = priced(&[]);
    let mut request = base_request(&usage, &unused_pricing, &extras);
    request.supplied_pricing = None;

    let tier: Tier = serde_json::from_str(r#"{"input_cost_per_token":"0.01","range":[0,1000]}"#)
        .expect("a fixture tier parses");
    let off_peak: OffPeakBlock =
        serde_json::from_str(r#"{"hours_utc":"00:00-23:59","output_cost_per_token":"0.02"}"#)
            .expect("a fixture off-peak block parses");
    let catalog_entry = PricedModel {
        record: model(&[
            ("input_cost_per_token", "\"0.1\""),
            ("output_cost_per_token", "\"0.2\""),
        ]),
        tiered_pricing: vec![tier],
        off_peak_pricing: Some(off_peak),
        off_peak_cache_read_present: false,
    };
    let lookup = CountingLookup::new(Some(catalog_entry));

    let result = generic_cost_per_token(&request, &lookup, &database)
        .expect("a catalog entry with tiers and an off-peak block still prices");

    assert_eq!(lookup.calls.get(), 1);
    assert_eq!(
        result.prompt_cost,
        Decimal::from_str_exact("5.0").expect("literal"),
        "the tier's input rate, not the flat record rate, must price the prompt side"
    );
    assert_eq!(
        result.completion_cost,
        Decimal::from_str_exact("2.0").expect("literal"),
        "the off-peak block's output rate, not the tier's fallback, must price the completion side"
    );
}

#[test]
fn exact_totals_apply_the_residency_then_vertex_uplift_to_both_sides() {
    let database = TimeZoneDatabase::from_env();
    let usage = fixture_usage(100, 50);
    let pricing = priced(&[
        ("input_cost_per_token", "\"0.001\""),
        ("output_cost_per_token", "\"0.002\""),
        ("regional_processing_uplift_multiplier_us", "\"1.1\""),
        ("regional_endpoint_uplift_multiplier", "\"1.05\""),
    ]);
    let extras = PromptExtras::default();
    let mut request = base_request(&usage, &pricing, &extras);
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
    let pricing = priced(&[
        ("input_cost_per_token", "\"0.01\""),
        ("output_cost_per_token", "\"0.01\""),
        ("regional_processing_uplift_multiplier_us", "\"2.0\""),
    ]);
    let extras = PromptExtras::default();
    let mut request = base_request(&usage, &pricing, &extras);
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
