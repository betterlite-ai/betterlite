#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters,
};
use betterlite_models::cost_usage::{CostUsage, PromptDetails};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionError};
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::{ModelInfo, TokenRateError};
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::BatchCostError;
use super::{BatchCostRequest, BatchCostResult, batch_cost_per_token};

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn model_info(json: &str) -> ModelInfo {
    serde_json::from_str(json).unwrap()
}

struct Fixture {
    prompt_tokens: i64,
    completion_tokens: i64,
    audio_tokens: i64,
    image_tokens: i64,
    video_tokens: i64,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    cached_tokens_details: Option<CachedTokenCounters>,
}

fn new_fixture() -> Fixture {
    Fixture {
        prompt_tokens: 0,
        completion_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        cache_creation_token_details: None,
        cached_tokens_details: None,
    }
}

impl Fixture {
    fn billing(&self) -> (CostUsage, PromptExtras) {
        let usage = CostUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: 0,
            prompt_details: Some(PromptDetails {
                text_tokens: None,
                audio_tokens: Some(self.audio_tokens),
                image_tokens: Some(self.image_tokens),
                video_tokens: Some(self.video_tokens),
                web_search_requests: None,
            }),
            completion_details: None,
            cache_read_tokens: self.cache_read_tokens,
            numeric_extras: BTreeMap::new(),
        };
        let extras = PromptExtras {
            cache_write_tokens: None,
            cache_creation_tokens: self.cache_creation_tokens,
            cache_creation_token_details: self.cache_creation_token_details,
            cached_tokens_details: self.cached_tokens_details,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
        };
        (usage, extras)
    }
}

struct Case {
    model: String,
    provider: Option<String>,
    residency: Option<String>,
    usage: CostUsage,
    extras: PromptExtras,
    supplied: Option<ModelInfo>,
}

fn case(
    model: &str,
    provider: Option<&str>,
    fixture: &Fixture,
    supplied: Option<ModelInfo>,
) -> Case {
    let (usage, extras) = fixture.billing();
    Case {
        model: model.to_owned(),
        provider: provider.map(str::to_owned),
        residency: None,
        usage,
        extras,
        supplied,
    }
}

impl Case {
    fn with_residency(mut self, residency: &str) -> Self {
        self.residency = Some(residency.to_owned());
        self
    }

    fn run(
        &self,
        lookup: &dyn PriceLookup,
        context: &ResolutionContext<'_>,
    ) -> Result<BatchCostResult, BatchCostError> {
        batch_cost_per_token(
            &BatchCostRequest {
                model: &self.model,
                custom_llm_provider: self.provider.as_deref(),
                usage: &self.usage,
                prompt_extras: &self.extras,
                supplied_model_info: self.supplied.as_ref(),
                data_residency: self.residency.as_deref(),
            },
            lookup,
            context,
        )
    }
}

struct ResolutionFixture {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl ResolutionFixture {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

struct RecordingLookup {
    record: Option<PriceRecord>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl RecordingLookup {
    fn returning(record: Option<PriceRecord>) -> Self {
        Self {
            record,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn never_called() -> Self {
        Self::returning(None)
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl PriceLookup for RecordingLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        self.record.clone()
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

#[test]
fn an_unresolvable_model_fails_before_supplied_prices_return_a_zero_pair() {
    let fixture = new_fixture();
    let supplied = model_info(r#"{"input_cost_per_token_batches": 1}"#);
    let resolution = ResolutionFixture::new();
    let error = case("vendor-model", None, &fixture, Some(supplied))
        .run(&RecordingLookup::never_called(), &resolution.context())
        .unwrap_err();
    assert!(matches!(
        error,
        BatchCostError::Resolution {
            source: ResolutionError::MissingProvider { .. }
        }
    ));
}

#[test]
fn complete_supplied_prices_skip_the_public_lookup() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.completion_tokens = 10;
    let supplied =
        model_info(r#"{"input_cost_per_token_batches": 0.5, "output_cost_per_token_batches": 1}"#);
    let lookup = RecordingLookup::returning(Some(model_info(
        r#"{"input_cost_per_token_batches": 9, "output_cost_per_token_batches": 9}"#,
    )));
    let resolution = ResolutionFixture::new();
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(&lookup, &resolution.context())
        .unwrap();
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(result.completion_cost, dec("10"));
    assert!(lookup.calls().is_empty());
}

#[test]
fn metadata_only_prices_are_replaced_by_the_whole_public_record() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    let supplied =
        model_info(r#"{"input_cost_per_audio_token_batches": 5, "input_cost_per_token": null}"#);
    let lookup =
        RecordingLookup::returning(Some(model_info(r#"{"input_cost_per_token_batches": 0.5}"#)));
    let resolution = ResolutionFixture::new();
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(&lookup, &resolution.context())
        .unwrap();
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(
        lookup.calls(),
        vec![("gpt-4o".to_owned(), Some("openai".to_owned()))]
    );
}

#[test]
fn a_failed_lookup_keeps_the_metadata_only_supplied_record() {
    let fixture = new_fixture();
    let supplied = model_info(
        r#"{"input_cost_per_audio_token_batches": 5, "input_cost_per_token_above_99999999999999999999_tokens_batches": 1}"#,
    );
    let lookup = RecordingLookup::returning(None);
    let resolution = ResolutionFixture::new();
    let error = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(&lookup, &resolution.context())
        .unwrap_err();
    assert!(matches!(
        error,
        BatchCostError::Rate(TokenRateError::Overflow)
    ));
    assert_eq!(
        lookup.calls(),
        vec![("gpt-4o".to_owned(), Some("openai".to_owned()))]
    );
}

#[test]
fn an_empty_supplied_record_without_a_public_answer_returns_the_zero_pair() {
    let fixture = new_fixture();
    let result = case("gpt-4o", Some("openai"), &fixture, Some(model_info("{}")))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn an_empty_public_record_returns_the_zero_pair() {
    let fixture = new_fixture();
    let lookup = RecordingLookup::returning(Some(PriceRecord::new()));
    let result = case("gpt-4o", Some("openai"), &fixture, None)
        .run(&lookup, &ResolutionFixture::new().context())
        .unwrap();
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn explicit_cache_rates_remove_and_price_their_tokens() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 1000;
    fixture.completion_tokens = 100;
    fixture.audio_tokens = 100;
    fixture.image_tokens = 50;
    fixture.video_tokens = 25;
    fixture.cache_read_tokens = Some(200);
    fixture.cache_creation_tokens = Some(100);
    let supplied = model_info(
        r#"{
            "input_cost_per_token_batches": 0.5,
            "output_cost_per_token_batches": 1.5,
            "cache_read_input_token_cost_batches": 0.25,
            "cache_creation_input_token_cost_batches": 0.125,
            "input_cost_per_audio_token_batches": 2,
            "input_cost_per_image_token_batches": 3,
            "input_cost_per_video_token_batches": 4
        }"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("775"));
    assert_eq!(result.completion_cost, dec("150"));
}

#[test]
fn ttl_write_splits_do_not_change_the_batch_prompt_path() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 1000;
    fixture.cache_creation_tokens = Some(100);
    fixture.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(40),
        ephemeral_1h_input_tokens: Some(60),
    });
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 0.5, "cache_creation_input_token_cost_batches": 0.25}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("475"));
}

#[test]
fn cached_tokens_stay_in_the_text_remainder_without_a_batch_cache_rate() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 1000;
    fixture.completion_tokens = 40;
    fixture.cache_read_tokens = Some(200);
    fixture.cache_creation_tokens = Some(100);
    let supplied =
        model_info(r#"{"input_cost_per_token_batches": 0.5, "output_cost_per_token_batches": 1}"#);
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("500"));
    assert_eq!(result.completion_cost, dec("40"));
}

#[test]
fn an_explicit_zero_cache_rate_removes_its_tokens_and_charges_nothing() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 1000;
    fixture.cache_read_tokens = Some(200);
    fixture.cache_creation_tokens = Some(100);
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 0.5, "cache_read_input_token_cost_batches": 0}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("400"));
}

#[test]
fn modalities_use_their_own_batch_rate_or_the_input_batch_rate() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.audio_tokens = 10;
    fixture.image_tokens = 20;
    fixture.video_tokens = 30;
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 0.5, "input_cost_per_audio_token_batches": 2}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("65"));
}

#[test]
fn a_negative_text_remainder_clamps_at_zero() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.audio_tokens = 60;
    fixture.image_tokens = 50;
    fixture.video_tokens = 10;
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 0.5, "input_cost_per_audio_token_batches": 0, "input_cost_per_image_token_batches": 1.5}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("80"));
}

#[test]
fn an_explicit_zero_input_batch_rate_still_takes_the_batch_path() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.audio_tokens = 10;
    fixture.completion_tokens = 5;
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 0, "input_cost_per_token": 0.5, "input_cost_per_audio_token_batches": 2}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("20"));
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn an_explicit_zero_output_batch_rate_beats_half_the_ordinary_rate() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.completion_tokens = 100;
    let supplied = model_info(
        r#"{"input_cost_per_token": 0.5, "output_cost_per_token_batches": 0, "output_cost_per_token": 1}"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("25"));
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn the_legacy_residual_keeps_its_negative_sign() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 10;
    fixture.cache_read_tokens = Some(50);
    let supplied = model_info(r#"{"input_cost_per_token": 0.5}"#);
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("-10"));
    assert_eq!(result.completion_cost, Decimal::ZERO);
}

#[test]
fn a_missing_legacy_cache_read_price_costs_nothing() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.cache_read_tokens = Some(50);
    let supplied = model_info(r#"{"input_cost_per_token": 0.5}"#);
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("12.5"));
}

#[rstest]
#[case::an_explicit_zero_write_price(
    r#"{"input_cost_per_token": 0.5, "cache_creation_input_token_cost": 0}"#
)]
#[case::a_missing_write_price(r#"{"input_cost_per_token": 0.5}"#)]
fn a_free_or_missing_legacy_write_price_falls_back_to_the_input_rate(#[case] supplied: &str) {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    fixture.cache_creation_tokens = Some(40);
    let result = case(
        "gpt-4o",
        Some("openai"),
        &fixture,
        Some(model_info(supplied)),
    )
    .run(
        &RecordingLookup::never_called(),
        &ResolutionFixture::new().context(),
    )
    .unwrap();
    assert_eq!(result.prompt_cost, dec("25"));
}

#[test]
fn a_truthy_ordinary_output_rate_is_charged_at_half() {
    let mut fixture = new_fixture();
    fixture.completion_tokens = 100;
    let supplied = model_info(r#"{"output_cost_per_token": 1}"#);
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, Decimal::ZERO);
    assert_eq!(result.completion_cost, dec("50"));
}

#[rstest]
#[case::us("us", "750", "150")]
#[case::eu_uppercased("EU", "1000", "200")]
#[case::unlisted("apac", "500", "100")]
fn only_the_residency_uplift_multiplies_both_sides(
    #[case] residency: &str,
    #[case] prompt_cost: &str,
    #[case] completion_cost: &str,
) {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 1000;
    fixture.completion_tokens = 100;
    let supplied = model_info(
        r#"{
            "input_cost_per_token_batches": 0.5,
            "output_cost_per_token_batches": 1,
            "input_cost_per_token_priority": 9,
            "off_peak_pricing": {"input_cost_per_token_batches": 0.25},
            "regional_processing_uplift_multiplier_eu": 2,
            "regional_processing_uplift_multiplier_us": 1.5
        }"#,
    );
    let result = case("gpt-4o", Some("openai"), &fixture, Some(supplied))
        .with_residency(residency)
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec(prompt_cost));
    assert_eq!(result.completion_cost, dec(completion_cost));
}

#[test]
fn the_resolved_provider_selects_the_inclusive_batch_thresholds() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 200_000;
    let supplied = model_info(
        r#"{"input_cost_per_token_batches": 1, "input_cost_per_token_above_200k_tokens_batches": 2}"#,
    );
    let result = case("xai/grok-2", None, &fixture, Some(supplied))
        .run(
            &RecordingLookup::never_called(),
            &ResolutionFixture::new().context(),
        )
        .unwrap();
    assert_eq!(result.prompt_cost, dec("400000"));
}

#[test]
fn the_lookup_sees_the_original_model_and_the_resolved_provider() {
    let mut fixture = new_fixture();
    fixture.prompt_tokens = 100;
    let lookup =
        RecordingLookup::returning(Some(model_info(r#"{"input_cost_per_token_batches": 0.5}"#)));
    let result = case("xai/grok-2", None, &fixture, None)
        .run(&lookup, &ResolutionFixture::new().context())
        .unwrap();
    assert_eq!(result.prompt_cost, dec("50"));
    assert_eq!(
        lookup.calls(),
        vec![("xai/grok-2".to_owned(), Some("xai".to_owned()))]
    );
}
