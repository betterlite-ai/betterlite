#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use crate::databricks_cost::{CostRequest, PROVIDER, cost_per_token, registry_key};
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::error::TokenCostError;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

fn scalar(text: &str) -> Scalar {
    serde_json::from_str(text).expect("a fixture scalar parses")
}

fn record() -> PriceRecord {
    BTreeMap::from([
        ("input_cost_per_token".to_owned(), scalar("\"0.000001\"")),
        ("output_cost_per_token".to_owned(), scalar("\"0.000002\"")),
        (
            "input_cost_per_token_priority".to_owned(),
            scalar("\"0.000004\""),
        ),
        (
            "output_cost_per_token_priority".to_owned(),
            scalar("\"0.000005\""),
        ),
    ])
}

struct MappedPrices;

impl PriceLookup for MappedPrices {
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
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        // only the mapped key under the databricks provider resolves, so a cost proves both
        (model == "databricks-dbrx-instruct" && custom_llm_provider == Some(PROVIDER)).then(|| {
            PricedModel {
                record: record(),
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            }
        })
    }
}

fn usage() -> CostUsage {
    CostUsage {
        prompt_tokens: 1_000,
        completion_tokens: 1_000,
        total_tokens: 2_000,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("the fixture moment parses")
}

#[rstest]
#[case::dbrx("dbrx-instruct", "databricks-dbrx-instruct")]
#[case::llama_70b(
    "meta-llama-3.1-70b-instruct",
    "databricks-meta-llama-3-1-70b-instruct"
)]
#[case::llama_405b(
    "meta-llama-3.1-405b-instruct",
    "databricks-meta-llama-3-1-405b-instruct"
)]
#[case::mixtral("mixtral-8x7b-instruct-v0.1", "databricks-mixtral-8x7b-instruct")]
#[case::bge("bge-large-en", "databricks-bge-large-en")]
#[case::gte("gte-large-en", "databricks-gte-large-en")]
#[case::llama_2("llama-2-70b-chat", "databricks-llama-2-70b-chat")]
fn the_legacy_table_replaces_the_whole_name(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(registry_key(model), expected);
}

#[rstest]
#[case::prefixed("databricks/dbrx-instruct", "databricks-dbrx-instruct")]
#[case::suffixed("dbrx-instruct-v2", "databricks-dbrx-instruct")]
#[case::unknown("databricks-mixtral-8x7b", "databricks-mixtral-8x7b")]
#[case::stripped_unknown("databricks/my-model", "my-model")]
fn a_name_is_matched_by_prefix_after_the_prefix_is_stripped(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(registry_key(model), expected);
}

#[test]
fn the_mapped_key_is_billed_with_the_databricks_provider() {
    let database = TimeZoneDatabase::from_env();
    let usage = usage();
    let request = CostRequest {
        model: "databricks/dbrx-instruct",
        usage: &usage,
        service_tier: None,
        moment: moment(),
    };
    let cost = cost_per_token(&request, &MappedPrices, &database).expect("the mapped key prices");
    assert_eq!(
        cost.prompt_cost,
        rust_decimal::Decimal::from_str("0.001").expect("literal")
    );
    assert_eq!(
        cost.completion_cost,
        rust_decimal::Decimal::from_str("0.002").expect("literal")
    );
}

#[test]
fn the_service_tier_selects_its_rate() {
    let database = TimeZoneDatabase::from_env();
    let usage = usage();
    let request = CostRequest {
        model: "dbrx-instruct",
        usage: &usage,
        service_tier: Some("priority"),
        moment: moment(),
    };
    let cost = cost_per_token(&request, &MappedPrices, &database).expect("the mapped key prices");
    assert_eq!(
        cost.prompt_cost,
        rust_decimal::Decimal::from_str("0.004").expect("literal")
    );
    assert_eq!(
        cost.completion_cost,
        rust_decimal::Decimal::from_str("0.005").expect("literal")
    );
}

#[test]
fn an_unknown_model_keeps_the_generic_error() {
    let database = TimeZoneDatabase::from_env();
    let usage = usage();
    let request = CostRequest {
        model: "databricks/my-model",
        usage: &usage,
        service_tier: None,
        moment: moment(),
    };
    let error = cost_per_token(&request, &MappedPrices, &database).expect_err("no price entry");
    assert!(matches!(error, TokenCostError::UnknownModel));
}
