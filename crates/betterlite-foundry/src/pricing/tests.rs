#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;

use super::additional::{additional_costs, router_additional_costs};
use super::error::PricingError;
use super::fee::router_flat_cost;
use super::names::{classify_router_name, router_fee_name};
use super::response::{ResponseCostRequest, cost_per_token};

struct CatalogLookup {
    entries: BTreeMap<String, PriceRecord>,
    calls: Cell<u32>,
}

impl CatalogLookup {
    fn new(entries: &[(&str, PriceRecord)]) -> Self {
        let mut catalog = Self {
            entries: BTreeMap::new(),
            calls: Cell::new(0),
        };
        for (model, record) in entries {
            catalog.entries.insert((*model).to_owned(), record.clone());
        }
        catalog
    }

    fn resolve(&self, model: &str, provider: Option<&str>) -> Option<PriceRecord> {
        self.calls.set(self.calls.get().saturating_add(1));
        provider
            .map(|provider_name| format!("{provider_name}/{model}"))
            .and_then(|prefixed| self.entries.get(&prefixed).cloned())
            .or_else(|| self.entries.get(model).cloned())
    }

    fn calls(&self) -> u32 {
        self.calls.get()
    }
}

impl PriceLookup for CatalogLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.resolve(model, custom_llm_provider)
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.resolve(model, custom_llm_provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

fn priced(entries: &[(&str, &str)]) -> PriceRecord {
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

fn amount(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a fixture decimal parses")
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

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

fn request<'fixture>(
    model: &'fixture str,
    usage: &'fixture CostUsage,
    extras: &'fixture PromptExtras,
) -> ResponseCostRequest<'fixture> {
    ResponseCostRequest {
        model,
        usage,
        prompt_extras: extras,
        response_time_ms: None,
        request_model: None,
        service_tier: None,
        moment: moment(),
    }
}

fn response_cost(
    lookup: &CatalogLookup,
    request: &ResponseCostRequest<'_>,
) -> Result<GenericCostResult, PricingError> {
    cost_per_token(request, lookup, &TimeZoneDatabase::from_env())
}

#[rstest]
#[case::hyphenated("azure-model-router", true)]
#[case::uppercase_hyphenated("MODEL-ROUTER", true)]
#[case::underscore_prefix("model_router/azure-gpt-4o", true)]
#[case::prefixed_fee_entry("AZURE_AI/Model-Router", true)]
#[case::embedded_substring("my-model-router-deployment", true)]
#[case::prefixed_deployment("azure_ai/model_router/deployment", true)]
#[case::missing_separator("modelrouter", false)]
#[case::plain("gpt-4o", false)]
#[case::empty("", false)]
fn router_shaped_names_accept_either_spelling(#[case] model: &str, #[case] expected: bool) {
    assert_eq!(classify_router_name(model).is_router_shaped(), expected);
}

#[rstest]
#[case::underscore_entry("model_router", true)]
#[case::hyphen_entry("model-router", true)]
#[case::uppercase_hyphen("MODEL-ROUTER", true)]
#[case::prefixed_underscore("azure_ai/model_router", true)]
#[case::prefixed_uppercase("AZURE_AI/Model-Router", true)]
#[case::double_prefix("azure_ai/azure_ai/model_router", false)]
#[case::suffixed_entry("model_router/deployment", false)]
#[case::prefixed_suffixed_entry("azure_ai/model_router/deployment", false)]
#[case::deployment_name("my-model-router", false)]
#[case::plain("plain-model", false)]
fn fee_entries_require_the_exact_name_after_one_prefix_strip(
    #[case] model: &str,
    #[case] expected: bool,
) {
    assert_eq!(classify_router_name(model).is_fee_entry(), expected);
}

#[rstest]
#[case::underscore("model_router", "model_router")]
#[case::hyphen("model-router", "model-router")]
#[case::uppercase_hyphen("MODEL-ROUTER", "model-router")]
#[case::uppercase_prefixed("Azure_AI/Model_Router", "model_router")]
#[case::double_prefix("azure_ai/azure_ai/model_router", "model_router")]
#[case::embedded_substring("my-model-router-deployment", "model_router")]
#[case::plain("plain-model", "model_router")]
fn fee_entry_names_keep_the_recognized_spelling_or_fall_back(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(classify_router_name(model).fee_lookup_name(), expected);
}

#[rstest]
#[case::exact_response_blocks_request("model_router", Some("model-router"), None)]
#[case::prefixed_exact_response_blocks_request(
    "azure_ai/model-router",
    Some("my-model-router"),
    None
)]
#[case::response_router_wins(
    "model_router/azure-gpt-4o",
    Some("request-model_router"),
    Some("model_router/azure-gpt-4o")
)]
#[case::request_router_when_response_plain(
    "plain-model",
    Some("request-model_router"),
    Some("request-model_router")
)]
#[case::both_plain("plain-model", Some("plain-request"), None)]
#[case::absent_request("plain-model", None, None)]
fn fee_name_prefers_the_response_then_the_request(
    #[case] model: &str,
    #[case] request_model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(router_fee_name(model, request_model), expected);
}

#[rstest]
#[case::positive_rate(&[("input_cost_per_token", "0.5")], 8, "4")]
#[case::absent_rate(&[("output_cost_per_token", "0.5")], 8, "0")]
#[case::zero_rate(&[("input_cost_per_token", "0")], 8, "0")]
#[case::negative_rate(&[("input_cost_per_token", "-0.5")], 8, "0")]
#[case::negative_tokens(&[("input_cost_per_token", "0.5")], -8, "-4")]
fn a_positive_input_rate_multiplies_prompt_tokens(
    #[case] entries: &[(&str, &str)],
    #[case] tokens: i64,
    #[case] expected: &str,
) {
    let lookup = CatalogLookup::new(&[("azure_ai/model_router", priced(entries))]);
    let fee =
        router_flat_cost("my-model-router", tokens, &lookup).expect("a mapped fee entry prices");
    assert_eq!(fee, amount(expected));
}

#[test]
fn a_plain_model_costs_nothing_without_a_lookup() {
    let lookup = CatalogLookup::new(&[("plain-model", priced(&[("input_cost_per_token", "5")]))]);
    let fee = router_flat_cost("plain-model", 8, &lookup).expect("a non-router name never fails");
    assert_eq!(fee, Decimal::ZERO);
    assert_eq!(lookup.calls(), 0);
}

#[test]
fn a_missing_fee_entry_propagates() {
    let lookup = CatalogLookup::new(&[]);
    let error =
        router_flat_cost("model_router", 8, &lookup).expect_err("an unmapped fee entry fails");
    assert!(matches!(
        error,
        PricingError::MissingFeeEntry {
            entry: "model_router"
        }
    ));
}

#[test]
fn the_fee_lookup_uses_the_recognized_entry_spelling() {
    let hyphen =
        CatalogLookup::new(&[("model-router", priced(&[("input_cost_per_token", "0.5")]))]);
    let fee = router_flat_cost("azure_ai/moDEL-router", 8, &hyphen)
        .expect("the hyphen spelling resolves");
    assert_eq!(fee, amount("4"));

    let underscore =
        CatalogLookup::new(&[("model_router", priced(&[("input_cost_per_token", "0.25")]))]);
    let fallback_fee = router_flat_cost("model_router/deployment", 8, &underscore)
        .expect("the fallback spelling resolves");
    assert_eq!(fallback_fee, amount("2"));
}

#[test]
fn the_fee_lookup_always_passes_the_azure_ai_provider() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    let fee =
        router_flat_cost("model_router", 8, &lookup).expect("the provider-prefixed entry resolves");
    assert_eq!(fee, amount("4"));
}

#[test]
fn overflowing_fee_multiplication_fails() {
    // Decimal::MAX as a quoted lexeme, which the shared coercion parses exactly
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "\"79228162514264337593543950335\"")]),
    )]);
    let error = router_flat_cost("model_router", 2, &lookup).expect_err("an overflowing fee fails");
    assert!(matches!(error, PricingError::Overflow));
}

#[test]
fn a_nonfinite_fee_rate_is_an_error() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "\"inf\"")]),
    )]);
    let error =
        router_flat_cost("model_router", 8, &lookup).expect_err("a nonfinite fee rate fails");
    assert!(matches!(
        error,
        PricingError::FeeRate {
            entry: "model_router",
            ..
        }
    ));
}

#[test]
fn the_router_hook_reports_a_positive_fee_under_the_exact_label() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    let costs = router_additional_costs("model_router", 8, &lookup)
        .expect("a priced fee reports")
        .expect("a positive fee is reported");
    assert_eq!(
        costs.into_iter().collect::<Vec<_>>(),
        vec![("Azure Model Router Flat Cost".to_owned(), amount("4"))]
    );
}

#[test]
fn the_router_hook_reports_nothing_for_a_plain_model() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    assert!(
        router_additional_costs("plain-model", 8, &lookup)
            .expect("a plain model never fails")
            .is_none()
    );
}

#[test]
fn the_router_hook_reports_no_zero_fee() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0")]),
    )]);
    assert!(
        router_additional_costs("model_router", 8, &lookup)
            .expect("a zero rate is not a failure")
            .is_none()
    );
}

#[test]
fn the_router_hook_reports_no_negative_fee() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    assert!(
        router_additional_costs("model_router", -8, &lookup)
            .expect("a negative prompt count is not a failure")
            .is_none()
    );
}

#[test]
fn the_router_hook_propagates_a_lookup_failure() {
    let lookup = CatalogLookup::new(&[]);
    let error =
        router_additional_costs("model_router", 8, &lookup).expect_err("the direct hook raises");
    assert!(matches!(error, PricingError::MissingFeeEntry { .. }));
}

#[rstest]
#[case::absent(None)]
#[case::foreign(Some("openai"))]
#[case::empty(Some(""))]
fn the_dispatcher_skips_absent_and_foreign_providers(#[case] provider: Option<&str>) {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    assert!(additional_costs("model_router", provider, 8, 4, &lookup).is_none());
    assert_eq!(lookup.calls(), 0);
}

#[test]
fn the_dispatcher_reports_the_router_fee_for_azure_ai() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    let costs = additional_costs("model_router", Some("azure_ai"), 8, 4, &lookup)
        .expect("a router model under azure_ai bills");
    assert_eq!(
        costs[super::additional::ROUTER_FLAT_COST_LABEL],
        amount("4")
    );
}

#[rstest]
#[case::plain_model("plain-model")]
#[case::agents("agents/model_router")]
#[case::claude("claude-sonnet")]
fn the_dispatcher_selects_only_the_model_router_config(#[case] model: &str) {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    assert!(additional_costs(model, Some("azure_ai"), 8, 4, &lookup).is_none());
    assert_eq!(lookup.calls(), 0);
}

#[test]
fn the_dispatcher_absorbs_a_missing_fee_entry() {
    let lookup = CatalogLookup::new(&[]);
    assert!(additional_costs("model_router", Some("azure_ai"), 8, 4, &lookup).is_none());
}

#[test]
fn a_routed_response_adds_the_fee_once() {
    let lookup = CatalogLookup::new(&[
        (
            "model_router/azure-gpt-4o",
            priced(&[
                ("input_cost_per_token", "0.25"),
                ("output_cost_per_token", "0.125"),
            ]),
        ),
        (
            "azure_ai/model_router",
            priced(&[("input_cost_per_token", "0.5")]),
        ),
    ]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let mut request = request("model_router/azure-gpt-4o", &usage, &extras);
    request.request_model = Some("azure_ai/model-router");
    let result = response_cost(&lookup, &request).expect("a routed response prices");
    assert_eq!(result.prompt_cost, amount("6"));
    assert_eq!(result.completion_cost, amount("0.5"));
}

#[test]
fn an_exact_fee_entry_response_adds_no_fee() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[
            ("input_cost_per_token", "0.5"),
            ("output_cost_per_token", "0.25"),
        ]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let mut request = request("model_router", &usage, &extras);
    request.request_model = Some("model-router");
    let result = response_cost(&lookup, &request).expect("an exact fee entry prices without a fee");
    assert_eq!(result.prompt_cost, amount("4"));
    assert_eq!(result.completion_cost, amount("1"));
    assert_eq!(lookup.calls(), 1);
}

#[test]
fn the_request_model_supplies_the_fee_for_a_plain_response() {
    let lookup = CatalogLookup::new(&[
        (
            "plain-model",
            priced(&[
                ("input_cost_per_token", "0.25"),
                ("output_cost_per_token", "0.125"),
            ]),
        ),
        (
            "azure_ai/model_router",
            priced(&[("input_cost_per_token", "0.5")]),
        ),
    ]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let mut request = request("plain-model", &usage, &extras);
    request.request_model = Some("request-model_router");
    let result = response_cost(&lookup, &request).expect("the request model supplies the fee");
    assert_eq!(result.prompt_cost, amount("6"));
    assert_eq!(result.completion_cost, amount("0.5"));
}

#[test]
fn an_unknown_router_response_prices_at_the_fee_alone() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let request = request("my-model-router-deployment", &usage, &extras);
    let result = response_cost(&lookup, &request).expect("a routed response falls back to the fee");
    assert_eq!(result.prompt_cost, amount("4"));
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn an_unknown_exact_fee_entry_response_costs_nothing() {
    let lookup = CatalogLookup::new(&[]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let request = request("model_router", &usage, &extras);
    let result = response_cost(&lookup, &request).expect("an unmapped fee entry charges nothing");
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn an_ordinary_response_pricing_failure_propagates() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[("input_cost_per_token", "0.5")]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let request = request("plain-model", &usage, &extras);
    let error = response_cost(&lookup, &request).expect_err("a plain unmapped model fails");
    assert!(matches!(error, PricingError::TokenCost(_)));
}

#[test]
fn an_unknown_router_response_with_an_unmapped_fee_fails() {
    let lookup = CatalogLookup::new(&[]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let request = request("model_router/gpt-4o", &usage, &extras);
    let error = response_cost(&lookup, &request)
        .expect_err("the swallowed response failure still leaves the fee failure");
    assert!(matches!(error, PricingError::MissingFeeEntry { .. }));
}

#[test]
fn an_overflowing_total_fails() {
    let lookup = CatalogLookup::new(&[
        ("my-model-router", priced(&[("input_cost_per_token", "1")])),
        (
            "azure_ai/model_router",
            priced(&[("input_cost_per_token", "\"79228162514264337593543950335\"")]),
        ),
    ]);
    let usage = usage(1, 0);
    let extras = PromptExtras::default();
    let request = request("my-model-router", &usage, &extras);
    let error = response_cost(&lookup, &request).expect_err("a total past the range fails");
    assert!(matches!(error, PricingError::Overflow));
}

#[test]
fn a_router_response_fails_when_the_fee_entry_is_missing() {
    let lookup = CatalogLookup::new(&[(
        "model_router/azure-gpt-4o",
        priced(&[
            ("input_cost_per_token", "0.25"),
            ("output_cost_per_token", "0.125"),
        ]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let request = request("model_router/azure-gpt-4o", &usage, &extras);
    let error = response_cost(&lookup, &request).expect_err("a missing fee entry fails");
    assert!(matches!(error, PricingError::MissingFeeEntry { .. }));
}

#[test]
fn the_service_tier_prices_the_response_model() {
    let lookup = CatalogLookup::new(&[(
        "plain-model",
        priced(&[
            ("input_cost_per_token", "0.25"),
            ("output_cost_per_token", "0.125"),
            ("input_cost_per_token_flex", "0.125"),
            ("output_cost_per_token_flex", "0.0625"),
        ]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let mut request = request("plain-model", &usage, &extras);
    request.service_tier = Some("flex");
    let result = response_cost(&lookup, &request).expect("a tiered model prices at its tier rate");
    assert_eq!(result.prompt_cost, amount("1"));
    assert_eq!(result.completion_cost, amount("0.25"));
}

#[test]
fn response_time_changes_nothing() {
    let lookup = CatalogLookup::new(&[(
        "azure_ai/model_router",
        priced(&[
            ("input_cost_per_token", "0.5"),
            ("output_cost_per_token", "0.25"),
        ]),
    )]);
    let usage = usage(8, 4);
    let extras = PromptExtras::default();
    let mut request = request("model_router", &usage, &extras);
    let without = response_cost(&lookup, &request).expect("a plain fee entry prices");
    request.response_time_ms = Some(amount("987654.25"));
    let with = response_cost(&lookup, &request).expect("a response time changes nothing");
    assert_eq!(with, without);
}
