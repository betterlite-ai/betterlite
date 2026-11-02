#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::{Provider, TokenForwardingRequest, cost_per_token};
use crate::cost_usage::CostUsage;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

fn usage(prompt_tokens: i64, completion_tokens: i64) -> CostUsage {
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

// Gates the priced lookup on the provider string the caller is expected to pass, proving dispatch by success or failure rather than by observing what reached the lookup.
struct Lookup {
    expected_provider: &'static str,
}

impl PriceLookup for Lookup {
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
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        if custom_llm_provider != Some(self.expected_provider) {
            return None;
        }
        let mut record = PriceRecord::new();
        record.insert(
            "input_cost_per_token".to_owned(),
            serde_json::from_str("0.001").expect("fixture scalar parses"),
        );
        record.insert(
            "output_cost_per_token".to_owned(),
            serde_json::from_str("0.002").expect("fixture scalar parses"),
        );
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

#[test]
fn cost_per_token_openai_uses_the_openai_provider_and_prices_both_sides() {
    let lookup = Lookup {
        expected_provider: "openai",
    };
    let database = TimeZoneDatabase::from_env();
    let fixture_usage = usage(100, 50);
    let request = TokenForwardingRequest {
        model: "gpt-x",
        usage: &fixture_usage,
        service_tier: None,
        moment: moment(),
    };
    let result = cost_per_token(
        &request,
        Provider::OpenAi {
            data_residency: None,
        },
        &lookup,
        &database,
    )
    .expect("priced model is found");
    assert_eq!(result.prompt_cost, Decimal::new(1, 1));
    assert_eq!(result.completion_cost, Decimal::new(1, 1));
}

#[test]
fn cost_per_token_azure_uses_the_azure_provider_and_ignores_response_time_ms() {
    let lookup = Lookup {
        expected_provider: "azure",
    };
    let database = TimeZoneDatabase::from_env();
    let fixture_usage = usage(100, 50);
    let request = TokenForwardingRequest {
        model: "gpt-x",
        usage: &fixture_usage,
        service_tier: None,
        moment: moment(),
    };
    let result = cost_per_token(
        &request,
        Provider::Azure {
            response_time_ms: Some(Decimal::from(5_000_i32)),
        },
        &lookup,
        &database,
    )
    .expect("priced model is found");
    assert_eq!(result.prompt_cost, Decimal::new(1, 1));
}

#[test]
fn cost_per_token_an_unregistered_model_raises() {
    struct NullLookup;
    impl PriceLookup for NullLookup {
        fn effective_model_info(&self, _: &str, _: Option<&str>) -> Option<PriceRecord> {
            None
        }
        fn raw_cost_map_entry(&self, _: &str) -> Option<PriceRecord> {
            None
        }
        fn effective_priced_model(&self, _: &str, _: Option<&str>) -> Option<PricedModel> {
            None
        }
    }
    let lookup = NullLookup;
    let database = TimeZoneDatabase::from_env();
    let fixture_usage = usage(10, 0);
    let request = TokenForwardingRequest {
        model: "ghost",
        usage: &fixture_usage,
        service_tier: None,
        moment: moment(),
    };
    let result = cost_per_token(
        &request,
        Provider::OpenAi {
            data_residency: None,
        },
        &lookup,
        &database,
    );
    result.unwrap_err();
}
