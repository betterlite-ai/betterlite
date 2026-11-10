#![cfg(test)]

use std::sync::Arc;

use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Version;

use crate::catalog::{BundledCatalog, ModelCatalog};
use crate::lookup::capabilities::{CapabilityFlag, CapabilityView};
use crate::lookup::info::{
    ModelId, ModelInfoError, ModelQuery, PtuBounds, declared_provider, find_entry, resolved_id,
    validated_ptu,
};
use crate::lookup::limits::{MaxTokensError, limits, max_output_tokens};
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
    }
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn query<'query>(model: &'query str, provider: Option<&'query str>) -> ModelQuery<'query> {
    ModelQuery { model, provider }
}

fn known_model() {
    let catalog = catalog();
    let found = find_entry(&catalog, &query("gpt-4o", Some("openai"))).unwrap();
    assert_eq!(found.name, "gpt-4o");
    assert_eq!(
        declared_provider(found, &query("gpt-4o", Some("openai"))).as_deref(),
        Some("openai")
    );
    assert_eq!(found.fields.metadata.mode, Some(Mode::Chat));
    let limits = limits(&catalog, &query("gpt-4o", Some("openai"))).unwrap();
    assert_eq!(limits.max_tokens, Some(0x4000));
    assert_eq!(limits.max_input_tokens, Some(128_000));
    assert_eq!(limits.max_output_tokens, Some(0x4000));
    let capabilities = CapabilityView::from_fields(&found.fields.capabilities);
    assert_eq!(capabilities.supports_vision, Some(true));
    assert_eq!(capabilities.supports_pdf_input, Some(true));
    assert_eq!(capabilities.supports_reasoning, None);
}

fn case_insensitive_name() {
    let catalog = catalog();
    let found = find_entry(&catalog, &query("GPT-4O", Some("openai"))).unwrap();
    assert_eq!(found.name, "gpt-4o");
}

fn provider_prefixed_key() {
    let catalog = catalog();
    let found = find_entry(&catalog, &query("gpt-4o", Some("azure"))).unwrap();
    assert_eq!(found.name, "azure/gpt-4o");
    assert_eq!(
        declared_provider(found, &query("gpt-4o", Some("azure"))).as_deref(),
        Some("azure")
    );
}

fn provider_mismatch() {
    let catalog = catalog();
    assert!(find_entry(&catalog, &query("gpt-4o", Some("anthropic"))).is_none());
    let error = limits(&catalog, &query("gpt-4o", Some("anthropic"))).unwrap_err();
    assert!(matches!(error, ModelInfoError::Unmapped { .. }));
}

fn dated_snapshot() {
    let catalog = catalog();
    let found = find_entry(&catalog, &query("gpt-4o-2027-01-01", None)).unwrap();
    assert_eq!(found.name, "gpt-4o");
}

fn unmapped_message() {
    let catalog = catalog();
    let error = limits(&catalog, &query("no-such-model", Some("openai"))).unwrap_err();
    assert_eq!(
        error.to_string(),
        "This model isn't mapped yet. model=no-such-model, custom_llm_provider=openai. \
         Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
    );
    let unconfigured = limits(&catalog, &query("no-such-model", None)).unwrap_err();
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
    assert_eq!(max_output_tokens(&catalog(), "gpt-4o").unwrap(), 0x4000);
}

fn output_tokens_fallback() {
    assert_eq!(
        max_output_tokens(&catalog(), "text-embedding-3-small").unwrap(),
        8_191
    );
    assert_eq!(
        max_output_tokens(&catalog(), "together-ai-8.1b-21b").unwrap(),
        1_000
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
    let found = find_entry(&catalog, &query("o3-mini", Some("openai"))).unwrap();
    let capabilities = CapabilityView::from_fields(&found.fields.capabilities);
    assert_eq!(capabilities.supports_web_search, None);
    assert_eq!(capabilities.supports_pdf_input, None);
    assert_eq!(capabilities.get(CapabilityFlag::WebSearch), None);
}

fn false_flag_stays_false() {
    let catalog = catalog();
    let found = find_entry(&catalog, &query("o3-mini", Some("openai"))).unwrap();
    let capabilities = CapabilityView::from_fields(&found.fields.capabilities);
    assert_eq!(capabilities.supports_vision, Some(false));
    assert_eq!(capabilities.get(CapabilityFlag::Vision), Some(false));
    assert_eq!(capabilities.supports_reasoning, Some(true));
}
