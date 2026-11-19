#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_anthropic::cache_aware_routing::services::PriceRequest;
use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::tiered_pricing::Scalar;
use chrono::DateTime;
use chrono::Utc;
use jiff::tz::TimeZoneDatabase;

use crate::cost::prompt_cache_scenarios::PriceEnvironment;

pub(crate) const MODEL: &str = "claude-x";
pub(crate) const DEPLOYMENT_ID: &str = "deployment-a";
const BILLING_SECONDS: i64 = 1_700_000_000;

pub(crate) fn moment() -> DateTime<Utc> {
    DateTime::from_timestamp(BILLING_SECONDS, 0).expect("a valid instant")
}

pub(crate) fn number(text: &str) -> Scalar {
    Scalar::Number(serde_json::Number::from_str(text).expect("the fixture is a number"))
}

pub(crate) fn text(value: &str) -> Scalar {
    Scalar::Text(value.to_owned())
}

pub(crate) fn record(fields: &[(&str, Scalar)]) -> PriceRecord {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn without(tariff: &PriceRecord, keys: &[&str]) -> PriceRecord {
    let mut tariff = tariff.clone();
    for key in keys {
        tariff.remove(*key);
    }
    tariff
}

// Scalar carries no PartialEq, so a record compares through its debug text instead.
pub(crate) fn rendered(record: &PriceRecord) -> BTreeMap<String, String> {
    record
        .iter()
        .map(|(key, value)| (key.clone(), format!("{value:?}")))
        .collect()
}

pub(crate) fn tokens(
    uncached: i64,
    read: i64,
    five_minute: i64,
    one_hour: i64,
) -> CacheTokenBuckets {
    CacheTokenBuckets {
        uncached_input_tokens: uncached,
        cache_read_input_tokens: read,
        cache_creation_5m_input_tokens: five_minute,
        cache_creation_1h_input_tokens: one_hour,
    }
}

pub(crate) fn flat_tariff() -> PriceRecord {
    record(&[
        ("input_cost_per_token", number("0.001")),
        ("output_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.001")),
        ("cache_creation_input_token_cost", number("0.001")),
        ("cache_creation_input_token_cost_above_1hr", number("0.001")),
    ])
}

pub(crate) fn request<'request>(
    tokens: &'request CacheTokenBuckets,
    completion_tokens: i64,
) -> PriceRequest<'request> {
    PriceRequest {
        model: MODEL,
        deployment_id: DEPLOYMENT_ID,
        tokens,
        completion_tokens,
    }
}

pub(crate) struct Lookup {
    raw: BTreeMap<String, PriceRecord>,
    effective: BTreeMap<String, PriceRecord>,
}

impl Lookup {
    pub(crate) fn new(raw: &[(&str, PriceRecord)]) -> Self {
        Self {
            raw: raw
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
            effective: BTreeMap::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_effective(mut self, model: &str, record: PriceRecord) -> Self {
        self.effective.insert(model.to_owned(), record);
        self
    }
}

impl PriceLookup for Lookup {
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
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

pub(crate) struct Prices {
    lookup: Lookup,
    database: TimeZoneDatabase,
}

impl Prices {
    pub(crate) fn new(lookup: Lookup) -> Self {
        Self {
            lookup,
            database: TimeZoneDatabase::from_env(),
        }
    }

    pub(crate) fn environment(&self) -> PriceEnvironment<'_> {
        PriceEnvironment {
            lookup: &self.lookup,
            database: &self.database,
        }
    }
}
