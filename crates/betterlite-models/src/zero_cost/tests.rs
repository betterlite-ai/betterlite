#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::TokenUsage;
use super::{
    AUDIO_INPUT_RATE, AUDIO_OUTPUT_RATE, MAX_PRICING_DEPTH, TEXT_INPUT_RATE, TEXT_OUTPUT_RATE,
    ZeroCostDiagnostic, ZeroCostReason, declares_a_rate, diagnose_zero_cost, used_pricing_keys,
    zero_cost_warning,
};
use crate::decisions::Json;

fn usage(prompt: i64, completion: i64, prompt_audio: i64, completion_audio: i64) -> TokenUsage {
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        prompt_audio_tokens: prompt_audio,
        completion_audio_tokens: completion_audio,
    }
}

fn text_usage() -> TokenUsage {
    usage(100, 50, 0, 0)
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn pricing(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn nested(entries: &[(&str, Json)]) -> Json {
    Json::Object(pricing(entries))
}

fn nest_cost(value: Json, depth: u32) -> Json {
    let mut nested = value;
    for _ in 0..depth {
        nested = nested_json(&nested);
    }
    nested
}

fn nested_json(value: &Json) -> Json {
    Json::Object(IndexMap::from([("nested_cost".to_owned(), value.clone())]))
}

fn diagnostic(reason: ZeroCostReason, keys: &[&'static str]) -> ZeroCostDiagnostic {
    ZeroCostDiagnostic {
        reason,
        pricing_model: "model".to_owned(),
        missing_pricing_keys: keys.to_vec(),
    }
}

#[test]
fn used_keys_are_empty_without_positive_use() {
    assert!(used_pricing_keys(&usage(0, 0, 0, 0)).is_empty());
    assert!(used_pricing_keys(&usage(-5, -2, 0, 0)).is_empty());
}

#[test]
fn audio_counts_are_only_used_while_positive() {
    assert_eq!(
        used_pricing_keys(&usage(5, 1, 10, 3)),
        vec![AUDIO_INPUT_RATE, AUDIO_OUTPUT_RATE]
    );
    assert_eq!(
        used_pricing_keys(&usage(100, 50, 10, 0)),
        vec![TEXT_INPUT_RATE, AUDIO_INPUT_RATE, TEXT_OUTPUT_RATE]
    );
}

#[test]
fn deliberately_free_used_rates_suppress_diagnostics() {
    let free = pricing(&[(TEXT_INPUT_RATE, number(0)), (TEXT_OUTPUT_RATE, number(0))]);
    assert_eq!(
        diagnose_zero_cost(&text_usage(), "model", &free, false),
        None
    );

    let partial = pricing(&[(TEXT_INPUT_RATE, number(0)), (TEXT_OUTPUT_RATE, number(5))]);
    assert_eq!(
        diagnose_zero_cost(&usage(1, 0, 0, 0), "model", &partial, false),
        None
    );
    assert_eq!(
        diagnose_zero_cost(&usage(1, 0, 0, 0), "model", &partial, true),
        None
    );
}

#[test]
fn missing_used_rate_names_keys_in_source_order() {
    let only_input = pricing(&[(TEXT_INPUT_RATE, number(5))]);

    assert_eq!(
        diagnose_zero_cost(&text_usage(), "model", &only_input, false),
        Some(diagnostic(
            ZeroCostReason::MissingPricingKey,
            &[TEXT_OUTPUT_RATE]
        ))
    );

    let audio_usage = usage(10, 10, 4, 3);
    assert_eq!(
        diagnose_zero_cost(&audio_usage, "model", &only_input, false),
        Some(diagnostic(
            ZeroCostReason::MissingPricingKey,
            &[AUDIO_INPUT_RATE, TEXT_OUTPUT_RATE, AUDIO_OUTPUT_RATE]
        ))
    );
}

#[test]
fn pricing_not_applied_when_every_used_rate_is_present() {
    let full = pricing(&[(TEXT_INPUT_RATE, number(5)), (TEXT_OUTPUT_RATE, number(6))]);

    assert_eq!(
        diagnose_zero_cost(&text_usage(), "model", &full, false),
        Some(diagnostic(ZeroCostReason::PricingNotApplied, &[]))
    );
}

#[test]
fn calculation_error_takes_priority_over_missing_key() {
    let partial = pricing(&[(TEXT_INPUT_RATE, number(5))]);

    assert_eq!(
        diagnose_zero_cost(&text_usage(), "model", &partial, true),
        Some(diagnostic(ZeroCostReason::CostCalculationError, &[]))
    );
    assert_eq!(
        diagnose_zero_cost(&text_usage(), "model", &IndexMap::new(), true),
        None
    );
}

#[rstest]
#[case::one_round(1)]
#[case::two_rounds(2)]
#[case::three_rounds(3)]
#[case::four_rounds(4)]
fn declared_rate_is_found_through_four_expansion_rounds(#[case] depth: u32) {
    let entry = pricing(&[(
        TEXT_INPUT_RATE,
        nest_cost(number(5), depth.saturating_sub(1)),
    )]);

    assert!(declares_a_rate(&entry));
}

#[test]
fn declared_rate_is_invisible_past_the_depth_boundary() {
    let entry = pricing(&[(TEXT_INPUT_RATE, nest_cost(number(5), MAX_PRICING_DEPTH))]);

    assert!(!declares_a_rate(&entry));
}

#[test]
fn case_sensitive_markers_and_sequences_declare_rates() {
    let upper = pricing(&[(TEXT_INPUT_RATE, nested(&[("COSTly", number(5))]))]);
    assert!(!declares_a_rate(&upper));

    let sequence = pricing(&[(
        TEXT_INPUT_RATE,
        Json::List(vec![
            nested(&[("nothing", number(1))]),
            nested(&[("x_cost", number(2))]),
        ]),
    )]);
    assert!(declares_a_rate(&sequence));
}

#[rstest]
#[case::numeric_string(Json::Text("5".to_owned()))]
#[case::boolean(Json::Bool(true))]
#[case::zero(number(0))]
#[case::negative(number(-3))]
fn numeric_strings_booleans_and_nonpositive_numbers_never_declare_rates(#[case] value: Json) {
    assert!(!declares_a_rate(&pricing(&[(TEXT_INPUT_RATE, value)])));
    assert!(declares_a_rate(&pricing(&[(TEXT_INPUT_RATE, number(1))])));
}

#[rstest]
#[case::missing_key(
    ZeroCostReason::MissingPricingKey,
    &[TEXT_OUTPUT_RATE],
    "pricing entry 'model' has no output_cost_per_token. Set the missing rate in the deployment's model_info or in the model cost map, or set every rate to 0 to mark the model free"
)]
#[case::not_applied(
    ZeroCostReason::PricingNotApplied,
    &[],
    "pricing entry 'model' declares non-zero rates for this usage, but the cost calculator returned $0"
)]
#[case::calculation_error(
    ZeroCostReason::CostCalculationError,
    &[],
    "cost calculation raised for pricing entry 'model', see response_cost_failure_debug_information"
)]
fn warning_text_matches_the_operator_message(
    #[case] reason: ZeroCostReason,
    #[case] keys: &[&'static str],
    #[case] cause: &str,
) {
    let warning = zero_cost_warning(
        &diagnostic(reason, keys),
        Some("group"),
        "model",
        Some("openai"),
        &text_usage(),
    );

    assert_eq!(
        warning,
        format!(
            "Billable request priced at $0 and logged as such (model_group=group model=model provider=openai prompt_tokens=100 completion_tokens=50): {cause}. Counted in litellm_zero_cost_requests_total{{reason=\"{}\"}}",
            reason.label()
        )
    );
}

#[test]
fn warning_falls_back_to_the_model_and_unknown_provider() {
    let warning = zero_cost_warning(
        &diagnostic(ZeroCostReason::PricingNotApplied, &[]),
        None,
        "model",
        None,
        &text_usage(),
    );

    assert!(warning.contains("model_group=model model=model provider=unknown"));
    assert!(
        warning.contains(
            "Counted in litellm_zero_cost_requests_total{reason=\"pricing_not_applied\"}"
        )
    );
}

#[test]
fn warning_uses_positive_token_counts_only() {
    let warning = zero_cost_warning(
        &diagnostic(ZeroCostReason::PricingNotApplied, &[]),
        None,
        "m",
        None,
        &usage(-4, 7, 0, 0),
    );

    assert!(warning.contains("prompt_tokens=0 completion_tokens=7"));
}
