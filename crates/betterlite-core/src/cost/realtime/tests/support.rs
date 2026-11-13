use std::collections::BTreeMap;
use std::convert::Infallible;
use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::usage_sum::Usage;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::cost::realtime::Event;
use crate::cost::realtime::pricing::RealtimeCostRequest;

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: &str) -> Json {
    Json::Number(Number::from_str(value).expect("the fixture is a number"))
}

pub(super) fn event(fields: &[(&str, Json)]) -> Event {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn object(fields: &[(&str, Json)]) -> Json {
    Json::Object(event(fields))
}

pub(super) fn scalar_number(value: &str) -> Scalar {
    Scalar::Number(Number::from_str(value).expect("the fixture is a number"))
}

pub(super) fn record(fields: &[(&str, Scalar)]) -> PriceRecord {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn token_usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("the fixture fits"),
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
}

pub(super) fn zero_usage() -> Usage {
    token_usage(0, 0)
}

#[derive(Default)]
pub(super) struct Catalog {
    pub(super) raw: BTreeMap<String, PriceRecord>,
    pub(super) effective: BTreeMap<String, PriceRecord>,
    calls: Mutex<Vec<String>>,
}

impl Catalog {
    pub(super) fn priced(entries: &[(&str, PriceRecord)]) -> Self {
        let raw: BTreeMap<String, PriceRecord> = entries
            .iter()
            .map(|(key, entry)| ((*key).to_owned(), entry.clone()))
            .collect();
        let effective = raw
            .iter()
            .map(|(key, entry)| {
                let mut resolved = entry.clone();
                resolved.insert("key".to_owned(), Scalar::Text(key.clone()));
                (key.clone(), resolved)
            })
            .collect();
        Self {
            raw,
            effective,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the lock").clone()
    }
}

impl PriceLookup for Catalog {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.effective.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.raw.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.calls.lock().expect("the lock").push(model.to_owned());
        self.effective.get(model).map(|entry| PricedModel {
            record: entry.clone(),
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

#[derive(Default)]
pub(super) struct Breakdown {
    pub(super) input_cost: Option<Decimal>,
    pub(super) output_cost: Option<Decimal>,
    pub(super) total_cost: Option<Decimal>,
    pub(super) tool_usage_cost: Option<Decimal>,
    pub(super) additional_costs: Option<BTreeMap<String, Decimal>>,
    pub(super) data_residency: Option<String>,
    pub(super) service_tier: Option<String>,
}

impl CostBreakdownTarget for Breakdown {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.input_cost = Some(input.input_cost);
        self.output_cost = Some(input.output_cost);
        self.total_cost = Some(input.total_cost);
        self.tool_usage_cost = Some(input.tool_usage_cost);
        self.additional_costs = input.additional_costs.clone();
        self.data_residency = input.data_residency.clone();
        self.service_tier = input.service_tier.clone();
        Ok(())
    }
}

pub(super) fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

pub(super) fn cost_request<'request>(
    results: &'request [Event],
    usage: &'request Usage,
    provider: &'request str,
    model: &'request str,
) -> RealtimeCostRequest<'request> {
    RealtimeCostRequest {
        results,
        combined_usage: usage,
        custom_llm_provider: provider,
        litellm_model_name: model,
        data_residency: None,
        custom_pricing_model: None,
        base_pricing_model: None,
        moment: DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed moment"),
    }
}

pub(super) fn priced(input: &str, output: &str) -> PriceRecord {
    record(&[
        ("input_cost_per_token", scalar_number(input)),
        ("output_cost_per_token", scalar_number(output)),
    ])
}

pub(super) fn session_event(event_type: &str, model: Option<&str>) -> Event {
    let mut session = event(&[]);
    if let Some(found) = model {
        session.insert("model".to_owned(), text(found));
    }
    event(&[
        ("type", text(event_type)),
        ("session", Json::Object(session)),
    ])
}

pub(super) fn response_usage(input: &str, output: &str) -> Json {
    object(&[
        ("input_tokens", number(input)),
        ("output_tokens", number(output)),
    ])
}
