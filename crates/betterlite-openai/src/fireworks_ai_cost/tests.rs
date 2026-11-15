#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::cost_usage::{CostUsage, PromptDetails};
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_rates::ModelInfo;
use chrono::DateTime;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::{FireworksCostError, LookupFailed};
use super::{
    FIREWORKS_AI_PROVIDER, FireworksPricingTiers, ModelInfoLookup, cost_per_token,
    get_base_model_for_pricing,
};

fn number(text: &str) -> Scalar {
    Scalar::Number(serde_json::from_str::<Number>(text).unwrap())
}

fn record(input: &str, output: &str, provider: &str) -> ModelInfo {
    BTreeMap::from([
        ("input_cost_per_token".to_owned(), number(input)),
        ("output_cost_per_token".to_owned(), number(output)),
        (
            "litellm_provider".to_owned(),
            Scalar::Text(provider.to_owned()),
        ),
    ])
}

fn usage(prompt: i64, completion: i64) -> CostUsage {
    CostUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

#[derive(Default)]
struct FakeLookup {
    entries: BTreeMap<(String, Option<String>), ModelInfo>,
    calls: Mutex<Vec<(String, Option<String>)>>,
    fail_all: bool,
}

impl FakeLookup {
    fn with(mut self, model: &str, provider: Option<&str>, info: ModelInfo) -> Self {
        self.entries
            .insert((model.to_owned(), provider.map(str::to_owned)), info);
        self
    }

    fn failing() -> Self {
        Self {
            entries: BTreeMap::new(),
            calls: Mutex::new(Vec::new()),
            fail_all: true,
        }
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ModelInfoLookup for FakeLookup {
    fn get(&self, model: &str, provider: Option<&str>) -> Result<ModelInfo, LookupFailed> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_owned(), provider.map(str::to_owned)));
        if self.fail_all {
            return Err(LookupFailed);
        }
        self.entries
            .get(&(model.to_owned(), provider.map(str::to_owned)))
            .cloned()
            .ok_or(LookupFailed)
    }
}

fn tiers() -> FireworksPricingTiers {
    FireworksPricingTiers::default()
}

#[rstest]
#[case::moe_56b("fireworks_ai/llama-v3p1-8x7b", "fireworks-ai-moe-up-to-56b")]
#[case::moe_176b("mixtral-8x22b", "fireworks-ai-56b-to-176b")]
#[case::moe_above_176b_falls_through("8x30b", "fireworks-ai-above-16b")]
#[case::moe_above_176b_without_b_digits(
    "accounts/fireworks/models/8x30b-instruct",
    "fireworks-ai-above-16b"
)]
#[case::up_to_4b("llama-v3p1-1b", "fireworks-ai-up-to-4b")]
#[case::just_above_4b("llama-v3p1-5b", "fireworks-ai-4.1b-to-16b")]
#[case::up_to_16b("llama-v3p1-16b", "fireworks-ai-4.1b-to-16b")]
#[case::just_above_16b("llama-v3p1-17b", "fireworks-ai-above-16b")]
#[case::first_size_match_wins("llama-v3-70b-instruct-lite-4b", "fireworks-ai-above-16b")]
#[case::uppercased("FIREWORKS_AI/LLAMA-V3P1-70B-INSTRUCT", "fireworks-ai-above-16b")]
#[case::provider_prefixed(
    "fireworks_ai/accounts/fireworks/models/llama-v3p1-70b",
    "fireworks-ai-above-16b"
)]
#[case::no_size("nomic-ai/nomic-embed-text-v1.5", "fireworks-ai-default")]
#[case::empty("", "fireworks-ai-default")]
fn get_base_model_for_pricing_maps_names_to_litellm_tiers(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(get_base_model_for_pricing(model, &tiers()), expected);
}

#[test]
fn get_base_model_for_pricing_honours_a_custom_tier_limit() {
    let raised = FireworksPricingTiers {
        up_to_4b: 100,
        up_to_16b: 200,
        moe_up_to_56b: 1000,
        moe_up_to_176b: 2000,
    };
    assert_eq!(
        get_base_model_for_pricing("llama-v3p1-70b", &raised),
        "fireworks-ai-up-to-4b"
    );
    assert_eq!(
        get_base_model_for_pricing("8x100b", &raised),
        "fireworks-ai-moe-up-to-56b"
    );
}

#[test]
fn cost_per_token_prices_the_provider_qualified_record() {
    let lookup = FakeLookup::default().with(
        "accounts/fireworks/models/llama-v3p1-70b",
        Some(FIREWORKS_AI_PROVIDER),
        record("0.000001", "0.000002", FIREWORKS_AI_PROVIDER),
    );
    let (prompt, completion) = cost_per_token(
        "accounts/fireworks/models/llama-v3p1-70b",
        &usage(8, 8),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap();
    assert_eq!(prompt, Decimal::new(8, 6));
    assert_eq!(completion, Decimal::new(16, 6));
    assert_eq!(
        lookup.calls(),
        [(
            "accounts/fireworks/models/llama-v3p1-70b".to_owned(),
            Some(FIREWORKS_AI_PROVIDER.to_owned()),
        )]
    );
}

#[test]
fn cost_per_token_falls_back_to_the_unprefixed_model_without_a_provider() {
    let lookup = FakeLookup::default().with(
        "accounts/fireworks/models/llama-v3p1-70b",
        None,
        record("0.000001", "0.000002", FIREWORKS_AI_PROVIDER),
    );
    let (prompt, _completion) = cost_per_token(
        "fireworks_ai/accounts/fireworks/models/llama-v3p1-70b",
        &usage(8, 0),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap();
    assert_eq!(prompt, Decimal::new(8, 6));
    assert_eq!(
        lookup.calls(),
        [
            (
                "fireworks_ai/accounts/fireworks/models/llama-v3p1-70b".to_owned(),
                Some(FIREWORKS_AI_PROVIDER.to_owned()),
            ),
            ("accounts/fireworks/models/llama-v3p1-70b".to_owned(), None,),
        ]
    );
}

#[test]
fn cost_per_token_falls_back_to_the_pricing_tier_with_the_fireworks_provider() {
    let lookup = FakeLookup::default().with(
        "fireworks-ai-above-16b",
        Some(FIREWORKS_AI_PROVIDER),
        record("0.0000009", "0.0000009", FIREWORKS_AI_PROVIDER),
    );
    let (prompt, completion) = cost_per_token(
        "accounts/fireworks/models/llama-v3p1-70b",
        &usage(8, 8),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap();
    assert_eq!(prompt, Decimal::new(72, 7));
    assert_eq!(completion, Decimal::new(72, 7));
    assert_eq!(
        lookup.calls()[2],
        (
            "fireworks-ai-above-16b".to_owned(),
            Some(FIREWORKS_AI_PROVIDER.to_owned()),
        )
    );
}

#[test]
fn cost_per_token_returns_the_third_lookup_failure() {
    let lookup = FakeLookup::failing();
    let error = cost_per_token(
        "accounts/fireworks/models/llama-v3p1-70b",
        &usage(8, 8),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap_err();
    assert!(matches!(error, FireworksCostError::Lookup { .. }));
    assert_eq!(lookup.calls().len(), 3);
}

fn provider_error(info: ModelInfo) -> FireworksCostError {
    let lookup = FakeLookup::default().with(
        "accounts/fireworks/models/x",
        Some(FIREWORKS_AI_PROVIDER),
        info,
    );
    cost_per_token(
        "accounts/fireworks/models/x",
        &usage(1, 1),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap_err()
}

#[test]
fn cost_per_token_needs_a_text_litellm_provider() {
    let missing = BTreeMap::from([("input_cost_per_token".to_owned(), number("0.000001"))]);
    assert!(matches!(
        provider_error(missing),
        FireworksCostError::Provider { .. }
    ));
    let null = BTreeMap::from([("litellm_provider".to_owned(), Scalar::Null)]);
    assert!(matches!(
        provider_error(null),
        FireworksCostError::Provider { .. }
    ));
    let numeric = BTreeMap::from([("litellm_provider".to_owned(), number("1"))]);
    assert!(matches!(
        provider_error(numeric),
        FireworksCostError::Provider { .. }
    ));
}

#[test]
fn cost_per_token_applies_the_fireworks_cache_read_default_once() {
    let lookup = FakeLookup::default().with(
        "accounts/fireworks/models/x",
        Some(FIREWORKS_AI_PROVIDER),
        record("0.000002", "0.000002", FIREWORKS_AI_PROVIDER),
    );
    let mut cached = usage(8, 0);
    cached.prompt_details = Some(PromptDetails {
        text_tokens: Some(4),
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
    });
    cached.cache_read_tokens = Some(4);
    let (prompt, _completion) = cost_per_token(
        "accounts/fireworks/models/x",
        &cached,
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap();
    assert_eq!(prompt, Decimal::new(12, 6));
}

#[test]
fn cost_per_token_surfaces_a_rate_error() {
    let lookup = FakeLookup::default().with(
        "accounts/fireworks/models/x",
        Some(FIREWORKS_AI_PROVIDER),
        BTreeMap::from([
            (
                "input_cost_per_token".to_owned(),
                Scalar::Text("inf".to_owned()),
            ),
            (
                "litellm_provider".to_owned(),
                Scalar::Text(FIREWORKS_AI_PROVIDER.to_owned()),
            ),
        ]),
    );
    let error = cost_per_token(
        "accounts/fireworks/models/x",
        &usage(8, 0),
        DateTime::UNIX_EPOCH,
        &tiers(),
        &lookup,
    )
    .unwrap_err();
    assert!(matches!(error, FireworksCostError::Cost(_)));
}

#[test]
fn supplied_pricing_only_answers_no_catalog_lookup() {
    let lookup = super::SuppliedPricingOnly;
    assert!(lookup.effective_model_info("m", Some("p")).is_none());
    assert!(lookup.raw_cost_map_entry("m").is_none());
    assert!(lookup.effective_priced_model("m", Some("p")).is_none());
}
