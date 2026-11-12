#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::error::AnthropicPricingError;
use super::{
    ANTHROPIC_PROVIDER, AnthropicPriceRequest, BedrockPriceRequest, TokenPriceRequest, token_cost,
};
use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolProjection;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::token_cost::error::TokenCostError;
use crate::token_cost::prompt_overlap::PromptExtras;

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a fixture decimal parses")
}

fn record(pairs: &[(&str, &str)]) -> PriceRecord {
    pairs
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
}

fn priced(record: PriceRecord) -> PricedModel {
    PricedModel {
        record,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    }
}

struct Lookup {
    priced: Option<PricedModel>,
    info: Option<PriceRecord>,
    priced_calls: Cell<u32>,
    saw_anthropic: Cell<bool>,
}

impl Lookup {
    fn new(priced: Option<PricedModel>, info: Option<PriceRecord>) -> Self {
        Self {
            priced,
            info,
            priced_calls: Cell::new(0),
            saw_anthropic: Cell::new(false),
        }
    }
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.info.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.priced_calls
            .set(self.priced_calls.get().saturating_add(1));
        if custom_llm_provider == Some(ANTHROPIC_PROVIDER) {
            self.saw_anthropic.set(true);
        }
        self.priced.clone()
    }
}

struct AnthropicFixture {
    usage: CostUsage,
    extras: PromptExtras,
    info: Option<PriceRecord>,
    entry: Option<PriceRecord>,
    projection: ServerToolProjection,
    service_tier: Option<&'static str>,
}

impl AnthropicFixture {
    fn request(&self) -> TokenPriceRequest<'_> {
        TokenPriceRequest::Anthropic(AnthropicPriceRequest {
            model: "fixture-model",
            usage: &self.usage,
            prompt_extras: &self.extras,
            supplied_model_info: self.info.as_ref(),
            provider_specific_entry: self.entry.as_ref(),
            service_tier: self.service_tier,
            projection: &self.projection,
            moment: DateTime::UNIX_EPOCH,
        })
    }
}

fn anthropic_fixture() -> AnthropicFixture {
    AnthropicFixture {
        usage: CostUsage {
            prompt_tokens: 4,
            completion_tokens: 2,
            total_tokens: 0,
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        extras: PromptExtras::default(),
        info: None,
        entry: None,
        projection: ServerToolProjection::default(),
        service_tier: None,
    }
}

fn base_prices() -> PriceRecord {
    record(&[
        ("input_cost_per_token", "0.25"),
        ("output_cost_per_token", "0.5"),
    ])
}

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn cost(request: &TokenPriceRequest<'_>, lookup: &Lookup) -> (Decimal, Decimal) {
    let result = token_cost(request, lookup, &database()).expect("pricing succeeds");
    (result.prompt_cost, result.completion_cost)
}

#[test]
fn supplied_model_info_bills_without_a_catalog_lookup() {
    let mut fixture = anthropic_fixture();
    fixture.info = Some(base_prices());
    let lookup = Lookup::new(None, None);
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("1"), decimal("1"))
    );
    assert_eq!(lookup.priced_calls.get(), 0);
}

#[test]
fn an_empty_supplied_record_still_skips_the_catalog() {
    let mut fixture = anthropic_fixture();
    fixture.info = Some(record(&[]));
    let lookup = Lookup::new(Some(priced(base_prices())), Some(base_prices()));
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (Decimal::ZERO, Decimal::ZERO)
    );
    assert_eq!(lookup.priced_calls.get(), 0);
}

#[test]
fn an_unsupplied_record_resolves_through_the_lookup_with_provider_anthropic() {
    let fixture = anthropic_fixture();
    let lookup = Lookup::new(Some(priced(base_prices())), Some(base_prices()));
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("1"), decimal("1"))
    );
    assert_eq!(lookup.priced_calls.get(), 1);
    assert!(lookup.saw_anthropic.get());
}

#[test]
fn the_service_tier_selects_tier_specific_rates() {
    let mut fixture = anthropic_fixture();
    fixture.info = Some(record(&[
        ("input_cost_per_token", "0.25"),
        ("output_cost_per_token", "0.5"),
        ("input_cost_per_token_priority", "0.5"),
        ("output_cost_per_token_priority", "1"),
    ]));
    fixture.service_tier = Some("priority");
    let lookup = Lookup::new(None, None);
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("2"), decimal("2"))
    );
}

#[test]
fn geo_and_fast_speed_surcharges_apply_to_supplied_info() {
    let mut fixture = anthropic_fixture();
    fixture.info = Some(base_prices());
    fixture.entry = Some(record(&[("us", "1.1"), ("fast", "2")]));
    fixture.projection = ServerToolProjection {
        server_tool_use: None,
        inference_geo: Some("us".to_owned()),
        speed: Some("fast".to_owned()),
    };
    let lookup = Lookup::new(None, None);
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("2.2"), decimal("2.2"))
    );
}

#[test]
fn an_unsupplied_record_resolves_the_effective_info_before_surcharging() {
    let mut fixture = anthropic_fixture();
    fixture.entry = Some(record(&[("us", "1.1"), ("fast", "2")]));
    fixture.projection = ServerToolProjection {
        server_tool_use: None,
        inference_geo: Some("us".to_owned()),
        speed: Some("fast".to_owned()),
    };
    let lookup = Lookup::new(Some(priced(base_prices())), Some(base_prices()));
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("2.2"), decimal("2.2"))
    );
}

#[test]
fn a_failed_effective_lookup_leaves_the_generic_totals() {
    let mut fixture = anthropic_fixture();
    fixture.entry = Some(record(&[("us", "1.1"), ("fast", "2")]));
    fixture.projection = ServerToolProjection {
        server_tool_use: None,
        inference_geo: Some("us".to_owned()),
        speed: Some("fast".to_owned()),
    };
    let lookup = Lookup::new(Some(priced(base_prices())), None);
    assert_eq!(
        cost(&fixture.request(), &lookup),
        (decimal("1"), decimal("1"))
    );
}

#[test]
fn an_unknown_model_error_propagates_before_the_surcharge_block() {
    let fixture = anthropic_fixture();
    let lookup = Lookup::new(None, None);
    let result = token_cost(&fixture.request(), &lookup, &database());
    assert!(matches!(
        result,
        Err(AnthropicPricingError::Token(TokenCostError::UnknownModel))
    ));
}

#[test]
fn the_bedrock_policy_bills_generic_rates_with_the_service_tier() {
    let fixture = anthropic_fixture();
    let lookup = Lookup::new(
        Some(priced(record(&[
            ("input_cost_per_token", "0.25"),
            ("output_cost_per_token", "0.5"),
            ("input_cost_per_token_priority", "0.5"),
            ("output_cost_per_token_priority", "1"),
        ]))),
        Some(record(&[])),
    );
    let request = TokenPriceRequest::Bedrock(BedrockPriceRequest {
        model: "fixture-model",
        usage: &fixture.usage,
        prompt_extras: &fixture.extras,
        service_tier: Some("priority"),
        moment: DateTime::UNIX_EPOCH,
    });
    assert_eq!(cost(&request, &lookup), (decimal("2"), decimal("2")));
}

#[test]
fn the_bedrock_policy_without_a_tier_uses_standard_rates() {
    let fixture = anthropic_fixture();
    let lookup = Lookup::new(Some(priced(base_prices())), None);
    let request = TokenPriceRequest::Bedrock(BedrockPriceRequest {
        model: "fixture-model",
        usage: &fixture.usage,
        prompt_extras: &fixture.extras,
        service_tier: None,
        moment: DateTime::UNIX_EPOCH,
    });
    assert_eq!(cost(&request, &lookup), (decimal("1"), decimal("1")));
}

#[test]
fn the_bedrock_policy_fails_for_an_unknown_model() {
    let fixture = anthropic_fixture();
    let lookup = Lookup::new(None, None);
    let request = TokenPriceRequest::Bedrock(BedrockPriceRequest {
        model: "fixture-model",
        usage: &fixture.usage,
        prompt_extras: &fixture.extras,
        service_tier: None,
        moment: DateTime::UNIX_EPOCH,
    });
    let result = token_cost(&request, &lookup, &database());
    assert!(matches!(
        result,
        Err(AnthropicPricingError::Token(TokenCostError::UnknownModel))
    ));
}
