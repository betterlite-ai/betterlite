#![cfg(test)]

use std::sync::Arc;

use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Version;

use crate::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use crate::lookup::capabilities::{CapabilityFlag, CapabilityView};
use crate::lookup::info::{
    ModelId, ModelInfoError, ModelQuery, PtuBounds, resolved_id, validated_ptu,
};
use crate::lookup::limits::{MaxTokensError, ModelLimits, max_output_tokens};
use crate::lookup::remap::remap;
use crate::lookup::resolve::{BedrockNames, Resolved, SplitModel, find_info, split_model};
use crate::metadata::identity::Mode;
use crate::ptu_pricing::{MAX_COST_PER_PTU_PER_HOUR, MAX_PTU_COUNT};

#[derive(Clone, Copy, Debug)]
enum Check {
    KnownModel,
    CaseInsensitiveName,
    ProviderPrefixedKey,
    ProviderMismatch,
    DatedSnapshot,
    UnmappedMessage,
    MissingId,
    NegativeIntegerId,
    TextId,
    PtuCountZero,
    PtuCountNegative,
    PtuCountOverLimit,
    PtuCountBounds,
    PtuCostRange,
    PtuWindowEqual,
    PtuWindowAfter,
    OutputTokensPreferOutput,
    OutputTokensFallBack,
    OutputTokensUnmapped,
    MissingFlagStaysNone,
    FalseFlagStaysFalse,
    PrefixedModelWithoutProvider,
    BedrockRoutingNames,
    BedrockOnlyStripsForBedrock,
    FillMissingBackfills,
    FillMissingSkipsOtherCases,
    FillMissingKeepsEntryValues,
    CatalogFillMissing,
    GeneralizationFallback,
    GeneralizationLosesToExactEntry,
    CatalogGeneralization,
    EntryWithoutProvider,
    MaxTokensExactKeyOnly,
    MaxTokensWithoutLimits,
    SplitModelProviders,
    HuggingfaceNeverFetches,
    AzureNamesAreRenamed,
    VertexBetaReadsAsVertex,
    VertexModelGetsCatalogPrefixOrSuffix,
    MantleRegionIsStripped,
    FireworksModelUsesCostKey,
    ProviderOverlayLaysOverEntry,
}

#[rstest]
#[case::known_model_info(Check::KnownModel)]
#[case::case_insensitive_model_name(Check::CaseInsensitiveName)]
#[case::provider_prefixed_catalog_key(Check::ProviderPrefixedKey)]
#[case::provider_mismatch_is_unmapped(Check::ProviderMismatch)]
#[case::dated_snapshot_suffix_is_stripped(Check::DatedSnapshot)]
#[case::unmapped_message_matches_litellm(Check::UnmappedMessage)]
#[case::missing_id_becomes_a_uuid(Check::MissingId)]
#[case::integer_id_becomes_its_decimal_string(Check::NegativeIntegerId)]
#[case::text_id_is_kept(Check::TextId)]
#[case::ptu_count_zero_is_rejected(Check::PtuCountZero)]
#[case::ptu_count_negative_is_rejected(Check::PtuCountNegative)]
#[case::ptu_count_over_limit_is_rejected(Check::PtuCountOverLimit)]
#[case::ptu_count_bounds_are_inclusive(Check::PtuCountBounds)]
#[case::ptu_cost_range_is_enforced(Check::PtuCostRange)]
#[case::ptu_window_equal_bounds_are_rejected(Check::PtuWindowEqual)]
#[case::ptu_window_after_is_accepted(Check::PtuWindowAfter)]
#[case::max_output_tokens_prefers_the_output_limit(Check::OutputTokensPreferOutput)]
#[case::max_output_tokens_falls_back_to_max_tokens(Check::OutputTokensFallBack)]
#[case::max_output_tokens_unmapped_errors(Check::OutputTokensUnmapped)]
#[case::missing_capability_stays_none(Check::MissingFlagStaysNone)]
#[case::false_capability_stays_false(Check::FalseFlagStaysFalse)]
#[case::prefixed_model_resolves_without_a_provider(Check::PrefixedModelWithoutProvider)]
#[case::bedrock_routing_names_reach_the_lookup(Check::BedrockRoutingNames)]
#[case::bedrock_routing_strip_is_bedrock_only(Check::BedrockOnlyStripsForBedrock)]
#[case::fill_missing_backfills_absent_fields(Check::FillMissingBackfills)]
#[case::fill_missing_skips_other_modes_and_providers(Check::FillMissingSkipsOtherCases)]
#[case::fill_missing_keeps_the_entry_values(Check::FillMissingKeepsEntryValues)]
#[case::pinned_catalog_fills_azure_gpt_5_chat(Check::CatalogFillMissing)]
#[case::generalization_answers_an_unmapped_model(Check::GeneralizationFallback)]
#[case::generalization_loses_to_an_exact_entry(Check::GeneralizationLosesToExactEntry)]
#[case::pinned_catalog_generalizes_claude_sonnet_9(Check::CatalogGeneralization)]
#[case::entry_without_provider_takes_the_requested_one(Check::EntryWithoutProvider)]
#[case::max_tokens_reads_exact_keys_only(Check::MaxTokensExactKeyOnly)]
#[case::max_tokens_of_a_mapped_model_without_limits_is_none(Check::MaxTokensWithoutLimits)]
#[case::split_model_stands_in_for_get_llm_provider(Check::SplitModelProviders)]
#[case::huggingface_model_never_fetches_its_config(Check::HuggingfaceNeverFetches)]
#[case::azure_names_are_renamed_before_the_lookup(Check::AzureNamesAreRenamed)]
#[case::vertex_ai_beta_reads_as_vertex_ai(Check::VertexBetaReadsAsVertex)]
#[case::vertex_model_gets_its_meta_prefix_or_latest_suffix(
    Check::VertexModelGetsCatalogPrefixOrSuffix
)]
#[case::bedrock_mantle_region_is_stripped(Check::MantleRegionIsStripped)]
#[case::fireworks_model_resolves_through_the_cost_key(Check::FireworksModelUsesCostKey)]
#[case::provider_info_overlays_the_entry_for_a_named_provider(Check::ProviderOverlayLaysOverEntry)]
fn model_capabilities_model_capability_tests_behavior(#[case] check: Check) {
    match check {
        Check::KnownModel => known_model(),
        Check::CaseInsensitiveName => case_insensitive_name(),
        Check::ProviderPrefixedKey => provider_prefixed_key(),
        Check::ProviderMismatch => provider_mismatch(),
        Check::DatedSnapshot => dated_snapshot(),
        Check::UnmappedMessage => unmapped_message(),
        Check::MissingId => missing_id(),
        Check::NegativeIntegerId => integer_id(),
        Check::TextId => text_id(),
        Check::PtuCountZero => ptu_count_errors(&[0]),
        Check::PtuCountNegative => ptu_count_errors(&[-1]),
        Check::PtuCountOverLimit => ptu_count_errors(&[1_000_001]),
        Check::PtuCountBounds => ptu_count_bounds(),
        Check::PtuCostRange => ptu_cost_range(),
        Check::PtuWindowEqual => ptu_window_equal(),
        Check::PtuWindowAfter => ptu_window_after(),
        Check::OutputTokensPreferOutput => output_tokens_prefer_output(),
        Check::OutputTokensFallBack => output_tokens_fallback(),
        Check::OutputTokensUnmapped => output_tokens_unmapped(),
        Check::MissingFlagStaysNone => missing_flag_stays_none(),
        Check::FalseFlagStaysFalse => false_flag_stays_false(),
        Check::PrefixedModelWithoutProvider => prefixed_model_without_provider(),
        Check::BedrockRoutingNames => bedrock_routing_names(),
        Check::BedrockOnlyStripsForBedrock => bedrock_only_strips_for_bedrock(),
        Check::FillMissingBackfills => fill_missing_backfills(),
        Check::FillMissingSkipsOtherCases => fill_missing_skips_other_cases(),
        Check::FillMissingKeepsEntryValues => fill_missing_keeps_entry_values(),
        Check::CatalogFillMissing => catalog_fill_missing(),
        Check::GeneralizationFallback => generalization_fallback(),
        Check::GeneralizationLosesToExactEntry => generalization_loses_to_exact_entry(),
        Check::CatalogGeneralization => catalog_generalization(),
        Check::EntryWithoutProvider => entry_without_provider(),
        Check::MaxTokensExactKeyOnly => max_tokens_exact_key_only(),
        Check::MaxTokensWithoutLimits => max_tokens_without_limits(),
        Check::SplitModelProviders => split_model_providers(),
        Check::HuggingfaceNeverFetches => huggingface_never_fetches(),
        Check::AzureNamesAreRenamed => azure_names_are_renamed(),
        Check::VertexBetaReadsAsVertex => vertex_beta_reads_as_vertex(),
        Check::VertexModelGetsCatalogPrefixOrSuffix => vertex_model_gets_catalog_prefix_or_suffix(),
        Check::MantleRegionIsStripped => mantle_region_is_stripped(),
        Check::FireworksModelUsesCostKey => fireworks_model_uses_cost_key(),
        Check::ProviderOverlayLaysOverEntry => provider_overlay_lays_over_entry(),
    }
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn query<'query>(model: &'query str, provider: Option<&'query str>) -> ModelQuery<'query> {
    ModelQuery { model, provider }
}

fn identity(model: &str) -> String {
    model.to_owned()
}

fn drop_converse(model: &str) -> String {
    model.strip_prefix("converse/").unwrap_or(model).to_owned()
}

fn drop_region(model: &str) -> String {
    ["apac.", "eu-west-1/"]
        .iter()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model)
        .to_owned()
}

fn drop_us_east(model: &str) -> String {
    model.strip_prefix("us-east-1/").unwrap_or(model).to_owned()
}

const IDENTITY: BedrockNames = BedrockNames {
    base_model: identity,
    strip_routing_prefix: identity,
    strip_mantle_region: identity,
};
const STRIPPING: BedrockNames = BedrockNames {
    base_model: drop_region,
    strip_routing_prefix: drop_converse,
    strip_mantle_region: drop_us_east,
};
const VERTEX: &[u8] = br#"{
  "vertex_ai/meta/llama-synth": {"litellm_provider": "vertex_ai-llama_models", "mode": "chat"},
  "vertex_ai/mistral-synth@latest": {"litellm_provider": "vertex_ai-mistral_models", "mode": "chat"},
  "vertex_ai/jamba-synth@latest": {"litellm_provider": "vertex_ai-ai21_models", "mode": "chat"},
  "vertex_ai/decoy-synth@latest": {"litellm_provider": "vertex_ai-language-models", "mode": "chat"}
}"#;
const SYNTHETIC: &[u8] = br#"{
  "zed-chat": {"litellm_provider": "acme", "mode": "chat"},
  "zed-responses": {"litellm_provider": "acme", "mode": "responses"},
  "zed-unmoded": {"litellm_provider": "acme"},
  "zed-embed": {"litellm_provider": "acme", "mode": "embedding"},
  "zed-other": {"litellm_provider": "other", "mode": "chat"},
  "zed-own": {"litellm_provider": "acme", "mode": "chat", "max_input_tokens": 5, "supports_reasoning": false},
  "bare-model": {"mode": "chat"},
  "fw-own": {"mode": "chat", "supports_function_calling": false},
  "fallback_generalizations": {"rules": [
    {"name": "fill", "pattern": "zed", "fill_missing_for_providers": ["acme"], "model_info": {"supports_reasoning": true, "max_input_tokens": 999}},
    {"name": "cap", "pattern": "cap-", "model_info": {"mode": "chat", "max_output_tokens": 7, "supports_vision": true}}
  ]}
}"#;

fn synthetic() -> ModelCatalog {
    ModelCatalog::parse(SYNTHETIC, UnknownFields::Reject).unwrap()
}

fn resolve(
    catalog: &ModelCatalog,
    names: &BedrockNames,
    model: &str,
    provider: Option<&str>,
) -> Option<Resolved> {
    find_info(catalog, names, &query(model, provider)).unwrap()
}

fn known_model() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "gpt-4o", Some("openai")).unwrap();
    assert_eq!(found.key, "gpt-4o");
    assert_eq!(found.provider.as_deref(), Some("openai"));
    assert_eq!(found.mode, Some(Mode::Chat));
    let limits = found.limits;
    assert_eq!(limits.max_tokens, Some(0x4000));
    assert_eq!(limits.max_input_tokens, Some(128_000));
    assert_eq!(limits.max_output_tokens, Some(0x4000));
    let capabilities = found.capabilities;
    assert_eq!(capabilities.supports_vision, Some(true));
    assert_eq!(capabilities.supports_pdf_input, Some(true));
    assert_eq!(capabilities.supports_reasoning, None);
}

fn case_insensitive_name() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "GPT-4O", Some("openai")).unwrap();
    assert_eq!(found.key, "gpt-4o");
}

fn provider_prefixed_key() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "gpt-4o", Some("azure")).unwrap();
    assert_eq!(found.key, "azure/gpt-4o");
    assert_eq!(found.provider.as_deref(), Some("azure"));
}

fn provider_mismatch() {
    let catalog = catalog();
    assert!(resolve(&catalog, &IDENTITY, "gpt-4o", Some("anthropic")).is_none());
}

fn dated_snapshot() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "gpt-4o-2027-01-01", None).unwrap();
    assert_eq!(found.key, "gpt-4o");
}

fn unmapped_message() {
    let catalog = catalog();
    assert!(resolve(&catalog, &IDENTITY, "no-such-model", Some("openai")).is_none());
    let error = ModelInfoError::unmapped(&query("no-such-model", Some("openai")));
    assert_eq!(
        error.to_string(),
        "This model isn't mapped yet. model=no-such-model, custom_llm_provider=openai. \
         Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
    );
    assert!(resolve(&catalog, &IDENTITY, "no-such-model", None).is_none());
    let unconfigured = ModelInfoError::unmapped(&query("no-such-model", None));
    assert!(
        unconfigured
            .to_string()
            .contains("custom_llm_provider=None.")
    );
}

fn missing_id() {
    let id = resolved_id(None);
    let parsed = uuid::Uuid::parse_str(&id).unwrap();
    assert_eq!(parsed.get_version(), Some(Version::Random));
}

fn integer_id() {
    assert_eq!(resolved_id(Some(ModelId::Integer(-42))), "-42");
    assert_eq!(resolved_id(Some(ModelId::Integer(42))), "42");
}

fn text_id() {
    assert_eq!(
        resolved_id(Some(ModelId::Text("deployment-id"))),
        "deployment-id"
    );
}

fn empty_ptu() -> PtuBounds {
    PtuBounds {
        ptu_count: None,
        cost_per_ptu_per_hour: None,
        ptu_effective_from: None,
        ptu_effective_to: None,
    }
}

fn ptu_count_errors(counts: &[i64]) {
    for count in counts {
        let mut bounds = empty_ptu();
        bounds.ptu_count = Some(*count);
        let error = validated_ptu(&bounds).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ptu_count must be a positive integer no greater than 1000000"
        );
    }
}

fn ptu_count_bounds() {
    let mut lower = empty_ptu();
    lower.ptu_count = Some(1);
    assert_eq!(validated_ptu(&lower).unwrap().count, Some(1));
    let mut upper = empty_ptu();
    upper.ptu_count = Some(i64::from(MAX_PTU_COUNT));
    assert_eq!(validated_ptu(&upper).unwrap().count, Some(MAX_PTU_COUNT));
}

fn ptu_cost_range() {
    for cost in [Decimal::ZERO, MAX_COST_PER_PTU_PER_HOUR] {
        let mut bounds = empty_ptu();
        bounds.cost_per_ptu_per_hour = Some(cost);
        let validated = validated_ptu(&bounds).unwrap();
        assert_eq!(validated.cost_per_ptu_per_hour, Some(cost));
    }
    let above_max = MAX_COST_PER_PTU_PER_HOUR
        .checked_add(Decimal::new(1, 2))
        .unwrap();
    for cost in [Decimal::new(-1, 2), above_max] {
        let mut bounds = empty_ptu();
        bounds.cost_per_ptu_per_hour = Some(cost);
        let error = validated_ptu(&bounds).unwrap_err();
        assert_eq!(
            error.to_string(),
            "cost_per_ptu_per_hour must be a finite number between 0 and 1000000.0"
        );
    }
}

fn ptu_window_equal() {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut bounds = empty_ptu();
    bounds.ptu_effective_from = Some(start);
    bounds.ptu_effective_to = Some(start);
    let error = validated_ptu(&bounds).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ptu_effective_to must be after ptu_effective_from"
    );
}

fn ptu_window_after() {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
    let mut bounds = empty_ptu();
    bounds.ptu_effective_from = Some(start);
    bounds.ptu_effective_to = Some(end);
    assert_eq!(validated_ptu(&bounds).unwrap().effective_to, Some(end));
    let mut start_only = empty_ptu();
    start_only.ptu_effective_from = Some(start);
    assert_eq!(validated_ptu(&start_only).unwrap().effective_to, None);
}

fn output_tokens_prefer_output() {
    assert_eq!(
        max_output_tokens(&catalog(), "gpt-4o").unwrap(),
        Some(0x4000)
    );
}

fn output_tokens_fallback() {
    assert_eq!(
        max_output_tokens(&catalog(), "text-embedding-3-small").unwrap(),
        Some(8_191)
    );
    assert_eq!(
        max_output_tokens(&catalog(), "together-ai-8.1b-21b").unwrap(),
        Some(1_000)
    );
}

fn output_tokens_unmapped() {
    let error = max_output_tokens(&catalog(), "no-such-model").unwrap_err();
    assert!(matches!(error, MaxTokensError::Unmapped { .. }));
    assert_eq!(
        error.to_string(),
        "Model no-such-model isn't mapped yet. Add it here - \
         https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
    );
}

fn missing_flag_stays_none() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "o3-mini", Some("openai")).unwrap();
    let capabilities = found.capabilities;
    assert_eq!(capabilities.supports_web_search, None);
    assert_eq!(capabilities.supports_pdf_input, None);
    assert_eq!(capabilities.get(CapabilityFlag::WebSearch), None);
}

fn false_flag_stays_false() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "o3-mini", Some("openai")).unwrap();
    let capabilities = found.capabilities;
    assert_eq!(capabilities.supports_vision, Some(false));
    assert_eq!(capabilities.get(CapabilityFlag::Vision), Some(false));
    assert_eq!(capabilities.supports_reasoning, Some(true));
}

const BEDROCK_SONNET: &str = "anthropic.claude-3-5-sonnet-20240620-v1:0";

fn prefixed_model_without_provider() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "openai/gpt-4o", None).unwrap();
    assert_eq!(found.key, "gpt-4o");
    assert_eq!(found.provider.as_deref(), Some("openai"));
    assert_eq!(found.limits.max_output_tokens, Some(0x4000));
    assert_eq!(
        max_output_tokens(&catalog, "openai/gpt-4o").unwrap(),
        Some(0x4000)
    );
}

fn bedrock_routing_names() {
    let catalog = catalog();
    let routed = [
        format!("converse/{BEDROCK_SONNET}"),
        format!("apac.{BEDROCK_SONNET}"),
        format!("eu-west-1/{BEDROCK_SONNET}"),
    ];
    for model in &routed {
        let found = resolve(&catalog, &STRIPPING, model, Some("bedrock")).unwrap();
        assert_eq!(found.key, BEDROCK_SONNET, "{model}");
        assert_eq!(found.provider.as_deref(), Some("bedrock"));
        assert!(resolve(&catalog, &IDENTITY, model, Some("bedrock")).is_none());
    }
}

fn bedrock_only_strips_for_bedrock() {
    let catalog = catalog();
    let model = format!("converse/{BEDROCK_SONNET}");
    let found = resolve(&catalog, &STRIPPING, &model, Some("bedrock_converse")).unwrap();
    assert_eq!(found.key, BEDROCK_SONNET);
    let other = resolve(&catalog, &STRIPPING, "converse/gpt-4o", Some("openai"));
    assert!(other.is_none_or(|unexpected| unexpected.key != "gpt-4o"));
}

fn fill_missing_backfills() {
    let catalog = synthetic();
    for model in ["zed-chat", "zed-responses", "zed-unmoded"] {
        let found = resolve(&catalog, &IDENTITY, model, Some("acme")).unwrap();
        assert_eq!(found.key, model);
        assert_eq!(found.limits.max_input_tokens, Some(999), "{model}");
        assert_eq!(found.capabilities.supports_reasoning, Some(true), "{model}");
    }
}

fn fill_missing_skips_other_cases() {
    let catalog = synthetic();
    let embedding = resolve(&catalog, &IDENTITY, "zed-embed", Some("acme")).unwrap();
    assert_eq!(embedding.key, "zed-embed");
    assert_eq!(embedding.limits.max_input_tokens, None);
    assert_eq!(embedding.capabilities.supports_reasoning, None);
    let other = resolve(&catalog, &IDENTITY, "zed-other", Some("other")).unwrap();
    assert_eq!(other.limits.max_input_tokens, None);
    assert_eq!(other.capabilities.supports_reasoning, None);
}

fn fill_missing_keeps_entry_values() {
    let catalog = synthetic();
    let found = resolve(&catalog, &IDENTITY, "zed-own", Some("acme")).unwrap();
    assert_eq!(found.limits.max_input_tokens, Some(5));
    assert_eq!(found.capabilities.supports_reasoning, Some(false));
}

fn catalog_fill_missing() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "azure/gpt-5-chat", Some("azure")).unwrap();
    assert_eq!(found.key, "azure/gpt-5-chat");
    assert_eq!(found.capabilities.supports_reasoning, Some(true));
    assert_eq!(found.capabilities.supports_vision, Some(true));
}

fn generalization_fallback() {
    let catalog = synthetic();
    let bare = resolve(&catalog, &IDENTITY, "cap-x", None).unwrap();
    assert_eq!(bare.key, "cap-x");
    assert_eq!(bare.provider, None);
    assert_eq!(bare.mode, Some(Mode::Chat));
    assert_eq!(bare.limits.max_output_tokens, Some(7));
    assert_eq!(bare.capabilities.supports_vision, Some(true));
    let owned = resolve(&catalog, &IDENTITY, "cap-x", Some("acme")).unwrap();
    assert_eq!(owned.key, "acme/cap-x");
    assert_eq!(owned.provider.as_deref(), Some("acme"));
    let filled = resolve(&catalog, &IDENTITY, "zed-new", Some("acme")).unwrap();
    assert_eq!(filled.key, "acme/zed-new");
    assert_eq!(filled.limits.max_input_tokens, Some(999));
}

fn generalization_loses_to_exact_entry() {
    let catalog = synthetic();
    assert!(resolve(&catalog, &IDENTITY, "zed-embed", Some("other")).is_none());
    assert!(resolve(&catalog, &IDENTITY, "zed-chat", Some("other")).is_none());
}

fn catalog_generalization() {
    let catalog = catalog();
    let found = resolve(&catalog, &IDENTITY, "claude-sonnet-9", Some("anthropic")).unwrap();
    assert_eq!(found.key, "anthropic/claude-sonnet-9");
    assert_eq!(found.provider.as_deref(), Some("anthropic"));
    assert_eq!(found.mode, Some(Mode::Chat));
    assert_eq!(found.limits.max_input_tokens, Some(200_000));
    assert_eq!(found.capabilities.supports_vision, Some(true));
    assert_eq!(found.capabilities.supports_function_calling, Some(true));
    assert_eq!(found.capabilities.supports_reasoning, Some(true));
}

fn entry_without_provider() {
    let catalog = synthetic();
    let requested = resolve(&catalog, &IDENTITY, "bare-model", Some("acme")).unwrap();
    assert_eq!(requested.key, "bare-model");
    assert_eq!(requested.provider.as_deref(), Some("acme"));
    let unrequested = resolve(&catalog, &IDENTITY, "bare-model", None).unwrap();
    assert_eq!(unrequested.provider, None);
}

fn max_tokens_exact_key_only() {
    let catalog = catalog();
    let error = max_output_tokens(&catalog, "GPT-4O").unwrap_err();
    assert!(matches!(&error, MaxTokensError::Unmapped { model } if model == "GPT-4O"));
    assert_eq!(
        max_output_tokens(&catalog, "amazon.nova-canvas-v1:0").unwrap(),
        None
    );
    assert_eq!(
        max_output_tokens(&catalog, "openai/gpt-4o").unwrap(),
        Some(0x4000)
    );
}

fn max_tokens_without_limits() {
    let catalog = synthetic();
    assert_eq!(max_output_tokens(&catalog, "zed-chat").unwrap(), None);
    assert_eq!(max_output_tokens(&catalog, "bare-model").unwrap(), None);
}

fn split_model_providers() {
    let catalog = catalog();
    let split = |model: &str| split_model(&catalog, model);
    let expected = |provider: Option<&str>, model: &str| SplitModel {
        provider: provider.map(str::to_owned),
        model: model.to_owned(),
    };
    assert_eq!(split("openai/gpt-4o"), expected(Some("openai"), "gpt-4o"));
    assert_eq!(split("gpt-4o"), expected(Some("openai"), "gpt-4o"));
    assert_eq!(
        split("claude-sonnet-9"),
        expected(Some("anthropic"), "claude-sonnet-9")
    );
    assert_eq!(split("no-such-model"), expected(None, "no-such-model"));
    assert_eq!(
        split("azure/command-a-03-2025"),
        expected(Some("openai"), "azure/command-a-03-2025")
    );
}

fn huggingface_never_fetches() {
    let catalog = catalog();
    let model = "huggingface/meta-llama/Llama-3-8B";
    let found = resolve(&catalog, &IDENTITY, model, None).unwrap();
    assert_eq!(found.key, model);
    assert_eq!(found.provider.as_deref(), Some("huggingface"));
    assert_eq!(found.mode, Some(Mode::Chat));
    assert_eq!(found.limits, ModelLimits::default());
    assert_eq!(found.capabilities, CapabilityView::default());
    assert_eq!(max_output_tokens(&catalog, model).unwrap(), None);
}

fn azure_names_are_renamed() {
    let catalog = catalog();
    let renamed = [
        ("azure/gpt-41", "gpt-4.1", "openai"),
        ("gpt-35-turbo", "azure/gpt-35-turbo", "azure"),
        ("ada", "azure/ada", "azure"),
    ];
    for (model, key, provider) in renamed {
        let found = resolve(&catalog, &IDENTITY, model, None).unwrap();
        assert_eq!(found.key, key, "{model}");
        assert_eq!(found.provider.as_deref(), Some(provider), "{model}");
    }
    assert_eq!(remap(&catalog, "gpt-4o", None), ("gpt-4o".to_owned(), None));
}

fn vertex_beta_reads_as_vertex() {
    let catalog = catalog();
    let found = resolve(
        &catalog,
        &IDENTITY,
        "gemini-2.5-pro",
        Some("vertex_ai_beta"),
    )
    .unwrap();
    assert_eq!(found.key, "gemini-2.5-pro");
    assert_eq!(found.provider.as_deref(), Some("vertex_ai-language-models"));
    let (_, provider) = remap(&catalog, "gemini-2.5-pro", Some("vertex_ai_beta"));
    assert_eq!(provider, Some("vertex_ai"));
    let (_, other) = remap(&catalog, "gemini-2.5-pro", Some("gemini"));
    assert_eq!(other, Some("gemini"));
}

fn vertex_model_gets_catalog_prefix_or_suffix() {
    let synthetic = ModelCatalog::parse(VERTEX, UnknownFields::Reject).unwrap();
    let remapped = |model: &str| remap(&synthetic, model, Some("vertex_ai")).0;
    assert_eq!(remapped("llama-synth"), "meta/llama-synth");
    assert_eq!(remapped("mistral-synth"), "mistral-synth@latest");
    assert_eq!(remapped("jamba-synth"), "jamba-synth@latest");
    assert_eq!(remapped("decoy-synth"), "decoy-synth");
    assert_eq!(remapped("unknown"), "unknown");
    assert_eq!(
        remap(&synthetic, "llama-synth", Some("acme")).0,
        "llama-synth"
    );
    assert_eq!(remap(&synthetic, "llama-synth", None).0, "llama-synth");
    let beta = Some("vertex_ai_beta");
    let found = resolve(&synthetic, &IDENTITY, "mistral-synth", beta).unwrap();
    assert_eq!(found.key, "vertex_ai/mistral-synth@latest");
    let real = catalog();
    let model = "llama-3.1-405b-instruct-maas";
    let llama = resolve(&real, &IDENTITY, model, Some("vertex_ai")).unwrap();
    assert_eq!(llama.key, "vertex_ai/meta/llama-3.1-405b-instruct-maas");
}

fn mantle_region_is_stripped() {
    let catalog = catalog();
    let model = "us-east-1/openai.gpt-oss-120b";
    let mantle_key = "bedrock_mantle/openai.gpt-oss-120b";
    let found = resolve(&catalog, &STRIPPING, model, Some("bedrock_mantle")).unwrap();
    assert_eq!(found.key, mantle_key);
    assert_eq!(found.provider.as_deref(), Some("bedrock_mantle"));
    let kept = resolve(&catalog, &IDENTITY, model, Some("bedrock_mantle"));
    assert!(kept.is_none_or(|unexpected| unexpected.key != mantle_key));
    let other = resolve(&catalog, &STRIPPING, model, Some("bedrock"));
    assert!(other.is_none_or(|unexpected| unexpected.key != mantle_key));
}

fn fireworks_model_uses_cost_key() {
    let catalog = catalog();
    let model = "deepseek-coder-v2-instruct";
    let found = resolve(&catalog, &IDENTITY, model, Some("fireworks_ai")).unwrap();
    assert_eq!(
        found.key,
        "fireworks_ai/accounts/fireworks/models/deepseek-coder-v2-instruct"
    );
}

fn provider_overlay_lays_over_entry() {
    let catalog = catalog();
    let xai = resolve(&catalog, &IDENTITY, "grok-build-0.1", Some("xai")).unwrap();
    assert_eq!(xai.key, "xai/grok-build-0.1");
    assert_eq!(xai.capabilities.supports_web_search, Some(true));
    assert_eq!(xai.capabilities.supports_reasoning, Some(true));
    let unnamed = resolve(&catalog, &IDENTITY, "xai/grok-build-0.1", None).unwrap();
    assert_eq!(unnamed.capabilities.supports_web_search, None);
    let fireworks =
        |model: &str| resolve(&catalog, &IDENTITY, model, Some("fireworks_ai")).unwrap();
    let coder = fireworks("deepseek-coder-v2-instruct").capabilities;
    assert_eq!(coder.supports_function_calling, Some(false));
    assert_eq!(coder.supports_prompt_caching, Some(true));
    assert_eq!(coder.supports_reasoning, None);
    let bare = fireworks("deepseek-r1").capabilities;
    assert_eq!(bare.supports_function_calling, Some(true));
    assert_eq!(bare.supports_prompt_caching, Some(true));
    let kimi = fireworks("kimi-k2p6").capabilities;
    assert_eq!(kimi.supports_reasoning, Some(true));
    assert_eq!(kimi.supports_vision, Some(true));
    let own = synthetic();
    let keyed = resolve(&own, &IDENTITY, "fw-own", Some("fireworks_ai")).unwrap();
    assert_eq!(keyed.capabilities.supports_function_calling, Some(false));
}
