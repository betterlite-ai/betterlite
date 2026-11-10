use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::sampling_convert::model_preferences::{
    ModelInfo, ModelPreferences, has_priorities, normalise, resolve_model_from_preferences,
    select_model_by_priority,
};

use super::fixtures::{hint, metrics, names, scored_catalog, with_priorities};

#[test]
fn resolve_weights_cost_and_intelligence_together() {
    let catalog = BTreeMap::from([
        (String::from("cheap-fast"), metrics(1, 1_000, 100)),
        (String::from("mid"), metrics(15, 8_000, 50)),
        (String::from("pricey-smart"), metrics(150, 64_000, 10)),
    ]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(
            Some(Decimal::new(5, 1)),
            None,
            Some(Decimal::new(5, 1)),
        )),
        &names(&["cheap-fast", "mid", "pricey-smart"]),
        None,
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "mid");
}

#[rstest]
#[case::cost(Some(Decimal::ONE), None, None, "cheap")]
#[case::speed(None, Some(Decimal::ONE), None, "fast")]
#[case::intelligence(None, None, Some(Decimal::ONE), "smart")]
fn resolve_picks_the_model_each_priority_prefers(
    #[case] cost: Option<Decimal>,
    #[case] speed: Option<Decimal>,
    #[case] intelligence: Option<Decimal>,
    #[case] expected: &str,
) {
    let preferences = with_priorities(cost, speed, intelligence);
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap", "fast", "smart"]),
        Some("default-model"),
        None,
        &scored_catalog(),
    )
    .expect("resolution");
    assert_eq!(resolved, expected);
}

#[rstest]
#[case::cost(Some(Decimal::ONE), None, None)]
#[case::speed(None, Some(Decimal::ONE), None)]
#[case::intelligence(None, None, Some(Decimal::new(5, 1)))]
fn has_priorities_accepts_a_positive_weight(
    #[case] cost: Option<Decimal>,
    #[case] speed: Option<Decimal>,
    #[case] intelligence: Option<Decimal>,
) {
    assert!(has_priorities(&with_priorities(cost, speed, intelligence)));
}

#[rstest]
#[case::absent(None, None, None)]
#[case::zero(Some(Decimal::ZERO), None, None)]
#[case::negative(Some(Decimal::NEGATIVE_ONE), None, None)]
#[case::zero_and_negative(Some(Decimal::ZERO), Some(Decimal::NEGATIVE_ONE), None)]
fn has_priorities_refuses_absent_zero_and_negative_weights(
    #[case] cost: Option<Decimal>,
    #[case] speed: Option<Decimal>,
    #[case] intelligence: Option<Decimal>,
) {
    assert!(!has_priorities(&with_priorities(cost, speed, intelligence)));
}

#[test]
fn resolve_returns_the_first_hint_that_matches() {
    let mut preferences = with_priorities(None, None, None);
    preferences.hints = vec![hint(Some("fast")), hint(Some("smart"))];
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap", "fast", "smart"]),
        Some("cheap"),
        None,
        &scored_catalog(),
    )
    .expect("resolution");
    assert_eq!(resolved, "fast");
}

#[test]
fn resolve_prefers_a_direct_hint_over_an_earlier_substring_match() {
    let mut preferences = with_priorities(None, None, None);
    preferences.hints = vec![hint(Some("smart"))];
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap-smart", "smart"]),
        None,
        None,
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "smart");
}

#[test]
fn resolve_matches_a_hint_as_a_case_insensitive_substring() {
    let mut preferences = with_priorities(None, None, None);
    preferences.hints = vec![hint(Some("SMART"))];
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap", "pricey-smart"]),
        None,
        None,
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "pricey-smart");
}

#[test]
fn resolve_skips_a_hint_without_a_name() {
    let mut preferences = with_priorities(None, None, None);
    preferences.hints = vec![hint(None), hint(Some("smart"))];
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap", "smart"]),
        None,
        None,
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "smart");
}

#[test]
fn resolve_skips_an_empty_hint_name() {
    let mut preferences = with_priorities(None, None, None);
    preferences.hints = vec![hint(Some("")), hint(Some("smart"))];
    let resolved = resolve_model_from_preferences(
        Some(&preferences),
        &names(&["cheap", "smart"]),
        None,
        None,
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "smart");
}

#[test]
fn resolve_breaks_a_priority_tie_towards_the_first_candidate() {
    let catalog = BTreeMap::from([
        (String::from("first"), metrics(2, 1_000, 10)),
        (String::from("second"), metrics(2, 1_000, 10)),
    ]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(Some(Decimal::ONE), None, None)),
        &names(&["first", "second"]),
        None,
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "first");
}

#[test]
fn resolve_scores_a_single_candidate() {
    let catalog = BTreeMap::from([(String::from("only"), metrics(3, 2_000, 5))]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(Some(Decimal::ONE), None, None)),
        &names(&["only"]),
        None,
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "only");
}

#[test]
fn resolve_skips_a_candidate_without_model_info() {
    let catalog = BTreeMap::from([(String::from("cheap"), metrics(1, 1_000, 10))]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(Some(Decimal::ONE), None, None)),
        &names(&["unknown", "cheap"]),
        Some("unknown"),
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "cheap");
}

fn raw_metrics(max_output_tokens: Option<u64>, max_tokens: Option<u64>) -> ModelInfo {
    ModelInfo {
        input_cost_per_token: None,
        output_cost_per_token: None,
        max_output_tokens,
        max_tokens,
        output_tokens_per_second: None,
    }
}

#[rstest]
#[case::max_tokens_only(None, Some(4_000))]
#[case::zero_max_output(Some(0), Some(4_000))]
fn resolve_falls_back_to_max_tokens_for_intelligence(
    #[case] max_output_tokens: Option<u64>,
    #[case] max_tokens: Option<u64>,
) {
    let catalog = BTreeMap::from([
        (
            String::from("fallback"),
            raw_metrics(max_output_tokens, max_tokens),
        ),
        (String::from("small"), raw_metrics(Some(100), None)),
    ]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(None, None, Some(Decimal::ONE))),
        &names(&["small", "fallback"]),
        None,
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "fallback");
}

#[test]
fn resolve_falls_through_when_every_score_stays_below_minus_one() {
    let catalog = BTreeMap::from([(String::from("only"), raw_metrics(Some(500), None))]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(
            Some(Decimal::from(-4_i32)),
            Some(Decimal::new(1, 1)),
            None,
        )),
        &names(&["only"]),
        Some("picked"),
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "picked");
}

#[test]
fn resolve_treats_only_negative_throughputs_as_no_speed_metric() {
    let catalog = BTreeMap::from([
        (String::from("slow"), metrics(1, 1_000, -20)),
        (String::from("slowest"), metrics(1, 1_000, -10)),
    ]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(None, Some(Decimal::ONE), None)),
        &names(&["slow", "slowest"]),
        Some("picked"),
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "slow");
}

#[test]
fn resolve_does_not_score_models_without_priorities() {
    let catalog = BTreeMap::from([(String::from("cheap"), metrics(1, 1_000, 10))]);
    let resolved = resolve_model_from_preferences(
        Some(&with_priorities(Some(Decimal::ZERO), None, None)),
        &names(&["unknown", "cheap"]),
        Some("picked"),
        None,
        &catalog,
    )
    .expect("resolution");
    assert_eq!(resolved, "picked");
}

#[test]
fn resolve_uses_the_default_model_before_the_first_available_model() {
    let resolved = resolve_model_from_preferences(
        None,
        &names(&["cheap", "fast"]),
        Some("default-model"),
        Some("fallback-model"),
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "default-model");
}

#[test]
fn resolve_uses_the_first_available_model_after_an_empty_default() {
    let resolved = resolve_model_from_preferences(
        None,
        &names(&["cheap", "fast"]),
        Some(""),
        Some("fallback-model"),
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "cheap");
}

#[test]
fn resolve_uses_the_configured_sampling_model_after_an_empty_default() {
    let resolved = resolve_model_from_preferences(
        None,
        &[],
        Some(""),
        Some("fallback-model"),
        &BTreeMap::new(),
    )
    .expect("resolution");
    assert_eq!(resolved, "fallback-model");
}

#[test]
fn resolve_fails_when_no_model_is_available() {
    let error = resolve_model_from_preferences(None, &[], Some(""), Some(""), &BTreeMap::new())
        .expect_err("no model");
    assert!(error.to_string().contains("default_mcp_sampling_model"));
}

#[test]
fn normalise_gives_equal_values_the_neutral_score() {
    let scores = normalise(&[Decimal::from(4_i32), Decimal::from(4_i32)], false).expect("scores");
    assert_eq!(scores, vec![Decimal::new(5, 1), Decimal::new(5, 1)]);
}

#[test]
fn normalise_inverts_the_cost_scores() {
    let scores = normalise(
        &[
            Decimal::from(1_i32),
            Decimal::from(6_i32),
            Decimal::from(11_i32),
        ],
        true,
    )
    .expect("scores");
    assert_eq!(
        scores,
        vec![Decimal::ONE, Decimal::new(5, 1), Decimal::ZERO]
    );
}

#[test]
fn normalise_of_no_values_is_empty() {
    assert_eq!(normalise(&[], true).expect("scores"), Vec::new());
}

#[test]
fn model_preferences_decode_from_the_mcp_wire() {
    let preferences: ModelPreferences = serde_json::from_str(
        r#"{"hints":[{"name":"fast"}],"costPriority":0.25,"speedPriority":null}"#,
    )
    .expect("preferences");
    assert_eq!(preferences.hints, vec![hint(Some("fast"))]);
    assert_eq!(preferences.cost_priority, Decimal::new(25, 2));
    assert_eq!(preferences.speed_priority, Decimal::ZERO);
}

#[test]
fn null_preferences_take_their_defaults() {
    let preferences: ModelPreferences =
        serde_json::from_str(r#"{"hints":null,"costPriority":null}"#).expect("preferences");
    assert_eq!(preferences, with_priorities(None, None, None));
}

#[test]
fn a_null_hint_name_decodes_as_an_empty_name() {
    let preferences: ModelPreferences =
        serde_json::from_str(r#"{"hints":[{"name":null},{"name":"fast"}]}"#).expect("preferences");
    assert_eq!(preferences.hints, vec![hint(None), hint(Some("fast"))]);
}

#[test]
fn select_model_by_priority_scores_without_the_resolver() {
    let selected = select_model_by_priority(
        &names(&["cheap", "fast", "smart"]),
        &with_priorities(None, None, Some(Decimal::ONE)),
        &scored_catalog(),
    )
    .expect("selection");
    assert_eq!(selected, Some(String::from("smart")));
}
