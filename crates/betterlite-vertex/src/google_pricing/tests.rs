#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::catalog::CatalogEntry;
use betterlite_models::entry::ModelFields;
use betterlite_models::price_fields::PriceFields;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::value::RawValue;

use super::character::{CharacterRequest, cost_per_character};
use super::delegate::{GenericTokenPricing, GenericTokenRequest};
use super::error::GooglePricingError;
use super::grounding::{cost_per_web_search_request, maps_grounding_cost};
use super::routing::{CostMetric, cost_router};
use super::studio;
use super::token::{TokenRequest, cost_per_token};
use super::usage::{GoogleUsage, ServerToolUse};

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

// every fixture goes through the B193 field and price parsers, so the decimal inputs are the ones production reads
fn model_record(source: &str) -> CatalogEntry {
    let raw = RawValue::from_string(source.to_owned()).unwrap();
    let (fields, _unknown) = ModelFields::parse(&raw).unwrap();
    let prices = PriceFields::parse(&raw).unwrap();
    CatalogEntry {
        name: "oracle-model".to_owned(),
        fields,
        prices,
    }
}

fn prompt_details(web_search: Option<i64>, maps: Option<i64>) -> PromptTokensDetails {
    PromptTokensDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: web_search,
        google_maps_grounding_requests: maps,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cached_tokens: None,
        cached_tokens_details: None,
    }
}

fn google_usage(
    prompt: i64,
    completion: i64,
    details: Option<PromptTokensDetails>,
    server: ServerToolUse,
) -> GoogleUsage {
    GoogleUsage {
        tokens: Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt.checked_add(completion).unwrap(),
            prompt_tokens_details: details,
            completion_tokens_details: None,
            numeric_extras: BTreeMap::new(),
        },
        server_tool_use: server,
    }
}

fn plain_usage(prompt: i64, completion: i64) -> GoogleUsage {
    google_usage(prompt, completion, None, ServerToolUse::default())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedCall {
    model: String,
    custom_llm_provider: String,
    service_tier: Option<String>,
    vertex_location: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
}

struct FixturePricing {
    uplift: Decimal,
    fail: bool,
    calls: Mutex<Vec<RecordedCall>>,
    uplift_locations: Mutex<Vec<Option<String>>>,
}

impl FixturePricing {
    fn new(uplift: Decimal) -> Self {
        Self {
            uplift,
            fail: false,
            calls: Mutex::new(Vec::new()),
            uplift_locations: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            uplift: Decimal::ONE,
            fail: true,
            calls: Mutex::new(Vec::new()),
            uplift_locations: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    fn uplift_locations(&self) -> Vec<Option<String>> {
        self.uplift_locations.lock().unwrap().clone()
    }

    fn record(&self, request: &GenericTokenRequest<'_>) {
        self.calls.lock().unwrap().push(RecordedCall {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.to_owned(),
            service_tier: request.service_tier.map(str::to_owned),
            vertex_location: request.vertex_location.map(str::to_owned),
            prompt_tokens: request.usage.prompt_tokens,
            completion_tokens: request.usage.completion_tokens,
        });
    }
}

impl GenericTokenPricing for FixturePricing {
    fn generic_cost_per_token(
        &self,
        request: &GenericTokenRequest<'_>,
    ) -> Result<GenericCostResult, TokenCostError> {
        self.record(request);
        if self.fail {
            return Err(TokenCostError::UnknownModel);
        }
        Ok(GenericCostResult {
            prompt_cost: Decimal::from(request.usage.prompt_tokens),
            completion_cost: Decimal::from(request.usage.completion_tokens),
        })
    }

    fn vertex_endpoint_uplift(
        &self,
        _record: &CatalogEntry,
        vertex_location: Option<&str>,
    ) -> Decimal {
        self.uplift_locations
            .lock()
            .unwrap()
            .push(vertex_location.map(str::to_owned));
        self.uplift
    }
}

fn character_request(
    prompt_characters: Option<Decimal>,
    completion_characters: Option<Decimal>,
    vertex_location: Option<&str>,
) -> CharacterRequest<'_> {
    CharacterRequest {
        model: "gemini-1.5-pro-002",
        custom_llm_provider: "vertex_ai",
        prompt_characters,
        completion_characters,
        service_tier: Some("priority"),
        vertex_location,
    }
}

fn token_request<'request>(
    service_tier: Option<&'request str>,
    vertex_location: Option<&'request str>,
) -> TokenRequest<'request> {
    TokenRequest {
        model: "gemini-1.5-pro-002",
        custom_llm_provider: "vertex_ai",
        service_tier,
        vertex_location,
    }
}

fn charges(result: Result<GenericCostResult, GooglePricingError>) -> (String, String) {
    let cost = result.unwrap();
    (
        cost.prompt_cost.normalize().to_string(),
        cost.completion_cost.normalize().to_string(),
    )
}

// a null price field is a B193 parse error, so an absent rate is left out of the fixture entirely
fn character_rates_source(input_rate: Option<&str>, output_rate: Option<&str>) -> String {
    let mut fields: Vec<String> = Vec::new();
    if let Some(rate) = input_rate {
        fields.push(format!(r#""input_cost_per_character": {rate}"#));
    }
    if let Some(rate) = output_rate {
        fields.push(format!(r#""output_cost_per_character": {rate}"#));
    }
    format!("{{{}}}", fields.join(", "))
}

const CHARACTER_RECORD: &str = r#"{
    "input_cost_per_character": 0.000000625,
    "input_cost_per_character_above_128k_tokens": 0.00000125,
    "output_cost_per_character": 0.0000025
}"#;

const TOKEN_RECORD: &str = r#"{
    "input_cost_per_token": 0.0000005,
    "input_cost_per_token_above_128k_tokens": 0.00000125,
    "output_cost_per_token": 0.0000025,
    "output_cost_per_token_above_128k_tokens": 0.000004
}"#;

#[rstest]
#[case::claude(
    "claude-3-5-sonnet@20240620",
    "vertex_ai",
    "completion",
    CostMetric::PerToken
)]
#[case::llama("meta/llama-3-70b", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::mistral("mistral-large", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::jamba("jamba-1.5", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::codestral("codestral-22b", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::gemma("gemma-2-9b", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::embedding("text-embedding-004", "vertex_ai", "embedding", CostMetric::PerToken)]
#[case::aembedding("text-embedding-004", "vertex_ai", "aembedding", CostMetric::PerToken)]
#[case::gemini_two("gemini-2.0-flash", "vertex_ai", "completion", CostMetric::PerToken)]
#[case::gemini_one("gemini-1.5-pro", "vertex_ai", "completion", CostMetric::PerCharacter)]
#[case::studio_provider("gemini-2.0-flash", "gemini", "completion", CostMetric::PerCharacter)]
#[case::uppercase_partner("Claude-3", "vertex_ai", "completion", CostMetric::PerCharacter)]
#[case::plural_embeddings(
    "text-embedding-004",
    "vertex_ai",
    "embeddings",
    CostMetric::PerCharacter
)]
fn cost_router_picks_the_metric_from_the_model_and_call_type(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] call_type: &str,
    #[case] expected: CostMetric,
) {
    assert_eq!(cost_router(model, provider, call_type), expected);
}

#[rstest]
#[case::at_the_limit("32000", "0.02")]
#[case::one_character_past("32001", "0.04000125")]
fn character_input_switches_to_the_above_128k_rate_past_32000_characters(
    #[case] characters: &str,
    #[case] expected: &str,
) {
    let pricing = FixturePricing::new(Decimal::ONE);
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &model_record(CHARACTER_RECORD),
        &plain_usage(100, 20),
        &character_request(Some(dec(characters)), Some(dec("40")), None),
    ));
    assert_eq!(prompt, expected);
    assert_eq!(completion, "0.0001");
    assert!(pricing.calls().is_empty());
}

#[rstest]
#[case::at_the_limit("32000", "0.08")]
#[case::one_character_past("32001", "0.00016")]
fn character_output_above_the_threshold_bills_completion_tokens(
    #[case] characters: &str,
    #[case] expected: &str,
) {
    let pricing = FixturePricing::new(Decimal::ONE);
    let entry = model_record(
        r#"{
            "input_cost_per_character": 0.000000625,
            "output_cost_per_character": 0.0000025,
            "output_cost_per_character_above_128k_tokens": 0.000004
        }"#,
    );
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &entry,
        &plain_usage(100, 40),
        &character_request(Some(dec("100")), Some(dec(characters)), None),
    ));
    assert_eq!(prompt, "0.0000625");
    assert_eq!(completion, expected);
}

#[rstest]
#[case::gemini_pro("gemini-pro")]
#[case::gemini_1_0_pro("gemini-1.0-pro")]
#[case::gemini_2("gemini-2")]
fn character_pricing_keeps_the_base_rate_for_fixed_price_models(#[case] model: &str) {
    let pricing = FixturePricing::new(Decimal::ONE);
    let mut request = character_request(Some(dec("32001")), Some(dec("40")), None);
    request.model = model;
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &model_record(CHARACTER_RECORD),
        &plain_usage(1, 40),
        &request,
    ));
    assert_eq!(prompt, "0.020000625");
    assert_eq!(completion, "0.0001");
}

#[rstest]
#[case::base_rate_only(Some("0.000000625"), Some("0.0000025"), "0.0000625", "0.0001")]
#[case::input_missing(None, Some("0.0000025"), "100", "0.0001")]
#[case::output_missing(Some("0.000000625"), None, "0.0000625", "20")]
#[case::both_missing(None, None, "100", "20")]
fn missing_character_rates_fall_back_to_token_pricing_per_side(
    #[case] input_rate: Option<&str>,
    #[case] output_rate: Option<&str>,
    #[case] expected_prompt: &str,
    #[case] expected_completion: &str,
) {
    let pricing = FixturePricing::new(Decimal::ONE);
    let source = character_rates_source(input_rate, output_rate);
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &model_record(&source),
        &plain_usage(100, 20),
        &character_request(Some(dec("100")), Some(dec("40")), None),
    ));
    assert_eq!(prompt, expected_prompt);
    assert_eq!(completion, expected_completion);
}

#[test]
fn a_missing_above_128k_character_rate_falls_back_for_that_side_only() {
    let pricing = FixturePricing::new(Decimal::ONE);
    let entry = model_record(
        r#"{"input_cost_per_character": 0.000000625, "output_cost_per_character": 0.0000025}"#,
    );
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &entry,
        &plain_usage(100, 20),
        &character_request(Some(dec("32001")), Some(dec("40")), None),
    ));
    assert_eq!(prompt, "100");
    assert_eq!(completion, "0.0001");
    assert_eq!(pricing.calls().len(), 1);
}

#[test]
fn absent_character_counts_fall_back_to_token_pricing() {
    let pricing = FixturePricing::new(Decimal::ONE);
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &model_record(CHARACTER_RECORD),
        &plain_usage(5, 7),
        &character_request(None, None, None),
    ));
    assert_eq!(prompt, "5");
    assert_eq!(completion, "7");
    let calls = pricing.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].service_tier.as_deref(), Some("priority"));
    assert_eq!(calls[0].vertex_location, None);
}

#[test]
fn the_character_uplift_applies_once_around_the_token_fallback() {
    let pricing = FixturePricing::new(dec("1.25"));
    let entry = model_record(
        r#"{"output_cost_per_character": 0.5, "regional_endpoint_uplift_multiplier": 1.25}"#,
    );
    let (prompt, completion) = charges(cost_per_character(
        &pricing,
        &entry,
        &plain_usage(5, 10),
        &character_request(None, Some(dec("10")), Some("us-east5")),
    ));
    assert_eq!(prompt, "6.25");
    assert_eq!(completion, "6.25");
    let calls = pricing.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].vertex_location, None);
    assert_eq!(
        pricing.uplift_locations(),
        vec![Some("us-east5".to_owned())]
    );
}

#[rstest]
#[case::prompt_past_the_limit(128_001, 40, "0.16000125", "0.0001")]
#[case::prompt_at_the_limit(128_000, 40, "0.064", "0.0001")]
#[case::completion_past_the_limit(10, 128_001, "0.000005", "0.512004")]
#[case::completion_at_the_limit(10, 128_000, "0.000005", "0.32")]
fn token_pricing_tests_each_side_against_128k_independently(
    #[case] prompt_tokens: i64,
    #[case] completion_tokens: i64,
    #[case] expected_prompt: &str,
    #[case] expected_completion: &str,
) {
    let pricing = FixturePricing::new(Decimal::ONE);
    let (prompt, completion) = charges(cost_per_token(
        &pricing,
        &model_record(TOKEN_RECORD),
        &plain_usage(prompt_tokens, completion_tokens),
        &token_request(None, None),
    ));
    assert_eq!(prompt, expected_prompt);
    assert_eq!(completion, expected_completion);
    assert!(pricing.calls().is_empty());
}

#[test]
fn token_pricing_enters_the_above_128k_path_when_only_one_side_has_a_rate() {
    let pricing = FixturePricing::new(Decimal::ONE);
    let entry = model_record(
        r#"{
            "input_cost_per_token": 0.0000005,
            "output_cost_per_token": 0.0000025,
            "output_cost_per_token_above_128k_tokens": 0.000004
        }"#,
    );
    let (prompt, completion) = charges(cost_per_token(
        &pricing,
        &entry,
        &plain_usage(100, 40),
        &token_request(None, None),
    ));
    assert_eq!(prompt, "0.00005");
    assert_eq!(completion, "0.0001");
    assert!(pricing.calls().is_empty());
}

#[test]
fn token_pricing_without_above_rates_delegates_every_request_field() {
    let pricing = FixturePricing::new(dec("1.25"));
    let entry = model_record(r#"{"input_cost_per_token": 0.1, "output_cost_per_token": 0.2}"#);
    let (prompt, completion) = charges(cost_per_token(
        &pricing,
        &entry,
        &plain_usage(5, 7),
        &token_request(Some("priority"), Some("us-east5")),
    ));
    assert_eq!(prompt, "5");
    assert_eq!(completion, "7");
    let calls = pricing.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gemini-1.5-pro-002");
    assert_eq!(calls[0].custom_llm_provider, "vertex_ai");
    assert_eq!(calls[0].service_tier.as_deref(), Some("priority"));
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-east5"));
    assert!(pricing.uplift_locations().is_empty());
}

#[test]
fn token_above_128k_pricing_applies_the_uplift_once() {
    let pricing = FixturePricing::new(dec("2"));
    let (prompt, completion) = charges(cost_per_token(
        &pricing,
        &model_record(TOKEN_RECORD),
        &plain_usage(128_001, 40),
        &token_request(None, Some("us-east5")),
    ));
    assert_eq!(prompt, "0.3200025");
    assert_eq!(completion, "0.00032");
    assert!(pricing.calls().is_empty());
    assert_eq!(
        pricing.uplift_locations(),
        vec![Some("us-east5".to_owned())]
    );
}

#[rstest]
#[case::token(false)]
#[case::character(true)]
fn delegate_failures_reach_the_caller(#[case] through_character: bool) {
    let pricing = FixturePricing::failing();
    let result = if through_character {
        cost_per_character(
            &pricing,
            &model_record(r#"{"input_cost_per_character": 0.5}"#),
            &plain_usage(5, 7),
            &character_request(Some(dec("10")), Some(dec("10")), None),
        )
    } else {
        cost_per_token(
            &pricing,
            &model_record(r#"{"input_cost_per_token": 0.1}"#),
            &plain_usage(5, 7),
            &token_request(None, None),
        )
    };
    assert!(matches!(
        result,
        Err(GooglePricingError::Token(TokenCostError::UnknownModel))
    ));
}

#[test]
fn studio_pricing_delegates_as_provider_gemini_without_a_location() {
    let pricing = FixturePricing::new(dec("1.25"));
    let (prompt, completion) = charges(studio::cost_per_token(
        &pricing,
        "gemini-1.5-pro-002",
        &plain_usage(5, 7),
        Some("flex"),
    ));
    assert_eq!(prompt, "5");
    assert_eq!(completion, "7");
    let calls = pricing.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].custom_llm_provider, "gemini");
    assert_eq!(calls[0].service_tier.as_deref(), Some("flex"));
    assert_eq!(calls[0].vertex_location, None);
}

fn search_usage(detail_count: Option<i64>, tool_count: Option<i64>) -> GoogleUsage {
    google_usage(
        3,
        0,
        Some(prompt_details(detail_count, None)),
        ServerToolUse {
            web_search_requests: tool_count,
        },
    )
}

fn maps_usage(requests: Option<i64>) -> GoogleUsage {
    google_usage(
        3,
        0,
        Some(prompt_details(None, requests)),
        ServerToolUse::default(),
    )
}

#[rstest]
#[case::grounded_prompt(Some(3), "0.01")]
#[case::no_requests(None, "0")]
fn web_search_charges_one_grounded_prompt_and_nothing_without_requests(
    #[case] web_search: Option<i64>,
    #[case] expected: &str,
) {
    let entry =
        model_record(r#"{"search_context_cost_per_query": {"search_context_size_medium": 0.01}}"#);
    let cost = cost_per_web_search_request(&search_usage(web_search, None), &entry).unwrap();
    assert_eq!(cost.normalize().to_string(), expected);
}

#[test]
fn web_search_per_query_bills_every_request_and_reads_only_the_medium_rate() {
    let entry = model_record(
        r#"{
            "search_context_cost_per_query": {"search_context_size_low": 0.5},
            "web_search_billing_unit": "per_query"
        }"#,
    );
    let cost = cost_per_web_search_request(&search_usage(Some(3), None), &entry).unwrap();
    assert_eq!(cost.normalize().to_string(), "0.105");
}

#[rstest]
#[case::prompt_details_win(Some(4), Some(9), "0.14")]
#[case::zero_details_fall_through(Some(0), Some(2), "0.07")]
#[case::absent_details_use_server_tool_use(None, Some(2), "0.07")]
#[case::no_counts_charge_nothing(None, None, "0")]
fn web_search_takes_the_prompt_detail_count_before_server_tool_use(
    #[case] web_search: Option<i64>,
    #[case] server: Option<i64>,
    #[case] expected: &str,
) {
    let entry = model_record(
        r#"{
            "search_context_cost_per_query": {"search_context_size_medium": 0.035},
            "web_search_billing_unit": "per_query"
        }"#,
    );
    let cost = cost_per_web_search_request(&search_usage(web_search, server), &entry).unwrap();
    assert_eq!(cost.normalize().to_string(), expected);
}

#[rstest]
#[case::absent(None)]
#[case::zero(Some(0))]
#[case::negative(Some(-2))]
fn maps_grounding_charges_nothing_without_a_positive_request_count(#[case] requests: Option<i64>) {
    let entry = model_record(r#"{"google_maps_grounding_cost_per_query": 0.014}"#);
    let cost = maps_grounding_cost(&maps_usage(requests), &entry).unwrap();
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn an_explicit_zero_maps_rate_does_not_fall_back_to_the_default_price() {
    let entry = model_record(r#"{"google_maps_grounding_cost_per_query": 0}"#);
    let cost = maps_grounding_cost(&maps_usage(Some(2)), &entry).unwrap();
    assert_eq!(cost, Decimal::ZERO);
}

#[rstest]
#[case::per_prompt_default(None, "0.014")]
#[case::per_prompt_configured(Some("per_prompt"), "0.014")]
#[case::per_query_configured(Some("per_query"), "0.042")]
fn maps_grounding_bills_one_prompt_or_every_query(
    #[case] unit: Option<&str>,
    #[case] expected: &str,
) {
    let source = unit.map_or_else(
        || r#"{"google_maps_grounding_cost_per_query": 0.014}"#.to_owned(),
        |value| {
            format!(
                r#"{{"google_maps_grounding_cost_per_query": 0.014, "web_search_billing_unit": "{value}"}}"#
            )
        },
    );
    let entry = model_record(&source);
    let cost = maps_grounding_cost(&maps_usage(Some(3)), &entry).unwrap();
    assert_eq!(cost.normalize().to_string(), expected);
}

#[test]
fn maps_grounding_defaults_to_the_per_prompt_price() {
    let cost = maps_grounding_cost(&maps_usage(Some(3)), &model_record("{}")).unwrap();
    assert_eq!(cost.normalize().to_string(), "0.025");
}

#[test]
fn maps_grounding_defaults_to_the_per_query_price_in_per_query_mode() {
    let entry = model_record(r#"{"web_search_billing_unit": "per_query"}"#);
    let cost = maps_grounding_cost(&maps_usage(Some(3)), &entry).unwrap();
    assert_eq!(cost.normalize().to_string(), "0.042");
}

#[test]
fn character_pricing_overflow_returns_a_typed_error() {
    let pricing = FixturePricing::new(Decimal::ONE);
    let result = cost_per_character(
        &pricing,
        &model_record(CHARACTER_RECORD),
        &plain_usage(1, 1),
        &character_request(Some(Decimal::MAX), Some(Decimal::ONE), None),
    );
    assert!(matches!(
        result,
        Err(GooglePricingError::Overflow {
            field: "character threshold"
        })
    ));
}
