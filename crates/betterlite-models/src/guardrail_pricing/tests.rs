#![cfg(test)]

use super::{
    GuardrailInformation, GuardrailInformationEntry, GuardrailPriceLookup, GuardrailPricingError,
    azure_prompt_shield_cost, bedrock_guardrail_cost_by_unit, billed_guardrail_cost_by_unit,
    guardrail_cost_total, guardrail_information_cost,
};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::str::FromStr as _;

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a fixture decimal parses")
}

fn usage(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), *value))
        .collect()
}

fn usable(pairs: &[(&str, &str)]) -> GuardrailPriceLookup {
    GuardrailPriceLookup::Usable(
        pairs
            .iter()
            .map(|(key, rate)| ((*key).to_owned(), decimal(rate)))
            .collect(),
    )
}

fn information(text: &str) -> GuardrailInformation {
    serde_json::from_str(text).expect("a fixture guardrail_information parses")
}

fn entry(text: &str) -> GuardrailInformationEntry {
    serde_json::from_str(text).expect("a fixture guardrail_information entry parses")
}

#[test]
fn bedrock_guardrail_cost_by_unit_prefers_the_regional_key_when_the_region_is_truthy() {
    let lookup = |key: &str| match key {
        "bedrock/us-east-1/guardrails" => Some(usable(&[("contentPolicyUnits", "2")])),
        "bedrock/guardrails" => Some(usable(&[("contentPolicyUnits", "9")])),
        _ => None,
    };
    let costs = bedrock_guardrail_cost_by_unit(
        lookup,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 3)]),
    )
    .expect("pricing resolves")
    .expect("a usable map was found");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("6")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_falls_through_a_malformed_regional_entry_to_global() {
    let lookup = |key: &str| match key {
        "bedrock/us-east-1/guardrails" => Some(GuardrailPriceLookup::Malformed),
        "bedrock/guardrails" => Some(usable(&[("contentPolicyUnits", "5")])),
        _ => None,
    };
    let costs = bedrock_guardrail_cost_by_unit(
        lookup,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 2)]),
    )
    .expect("pricing resolves")
    .expect("the global entry is used");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("10")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_lets_a_valid_empty_regional_map_block_global_fallback() {
    let lookup = |key: &str| match key {
        "bedrock/us-east-1/guardrails" => Some(GuardrailPriceLookup::Usable(BTreeMap::new())),
        "bedrock/guardrails" => Some(usable(&[("contentPolicyUnits", "5")])),
        _ => None,
    };
    let costs = bedrock_guardrail_cost_by_unit(
        lookup,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 2)]),
    )
    .expect("pricing resolves")
    .expect("the empty regional map still wins");
    assert_eq!(costs["contentPolicyUnits"], None);
}

#[test]
fn bedrock_guardrail_cost_by_unit_ignores_the_regional_key_for_a_falsy_region() {
    let lookup = |key: &str| match key {
        "bedrock/guardrails" => Some(usable(&[("contentPolicyUnits", "5")])),
        _ => Some(GuardrailPriceLookup::Malformed),
    };
    let costs =
        bedrock_guardrail_cost_by_unit(lookup, Some(""), &usage(&[("contentPolicyUnits", 1)]))
            .expect("pricing resolves")
            .expect("the global entry is used directly");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("5")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_returns_none_when_no_entry_is_usable_anywhere() {
    let lookup = |_key: &str| Some(GuardrailPriceLookup::Malformed);
    let costs = bedrock_guardrail_cost_by_unit(lookup, None, &usage(&[("contentPolicyUnits", 1)]))
        .expect("pricing resolves");
    assert!(costs.is_none());
}

#[test]
fn bedrock_guardrail_cost_by_unit_distinguishes_an_explicit_zero_rate_from_an_unpriced_counter() {
    let lookup = |_key: &str| Some(usable(&[("sensitiveInformationPolicyFreeUnits", "0")]));
    let costs = bedrock_guardrail_cost_by_unit(
        lookup,
        None,
        &usage(&[
            ("sensitiveInformationPolicyFreeUnits", 5),
            ("wordPolicyUnits", 5),
        ]),
    )
    .expect("pricing resolves")
    .expect("a usable map was found");
    assert_eq!(
        costs["sensitiveInformationPolicyFreeUnits"],
        Some(Decimal::ZERO)
    );
    assert_eq!(costs["wordPolicyUnits"], None);
}

#[test]
fn bedrock_guardrail_cost_by_unit_propagates_a_multiplication_overflow() {
    let lookup = |_key: &str| {
        Some(usable(&[(
            "contentPolicyUnits",
            "79228162514264337593543950335",
        )]))
    };
    let error = bedrock_guardrail_cost_by_unit(lookup, None, &usage(&[("contentPolicyUnits", 2)]))
        .expect_err("the product overflows Decimal");
    assert!(matches!(error, GuardrailPricingError::CostOverflow));
}

#[rstest]
#[case::absent_map(None, "0")]
#[case::known_and_unknown_counters(Some(vec![("a", Some("1.5")), ("b", None)]), "1.5")]
fn guardrail_cost_total_sums_known_values_and_treats_an_absent_map_as_zero(
    #[case] entries: Option<Vec<(&str, Option<&str>)>>,
    #[case] expected: &str,
) {
    let map = entries.map(|pairs| {
        pairs
            .into_iter()
            .map(|(key, rate)| (key.to_owned(), rate.map(decimal)))
            .collect::<BTreeMap<_, _>>()
    });
    let total = guardrail_cost_total(map.as_ref()).expect("sum succeeds");
    assert_eq!(total, decimal(expected));
}

#[test]
fn guardrail_cost_total_propagates_an_addition_overflow() {
    let map = BTreeMap::from([
        (
            "a".to_owned(),
            Some(decimal("79228162514264337593543950335")),
        ),
        ("b".to_owned(), Some(decimal("1"))),
    ]);
    let error = guardrail_cost_total(Some(&map)).expect_err("the sum overflows Decimal");
    assert!(matches!(error, GuardrailPricingError::CostOverflow));
}

#[rstest]
#[case::free_tier_ignores_price(Some("free"), Some("9"), 10, "0")]
#[case::missing_price_but_not_free(None, None, 10, "")]
#[case::priced_with_records(None, Some("3"), 2000, "6")]
#[case::priced_with_missing_count(None, Some("3"), 0, "0")]
fn azure_prompt_shield_guardrail_cost_distinguishes_free_missing_and_priced(
    #[case] cost_tier: Option<&str>,
    #[case] price: Option<&str>,
    #[case] text_records: i64,
    #[case] expected: &str,
) {
    let usage = usage(&[(super::AZURE_TEXT_RECORD_UNIT, text_records)]);
    let cost =
        azure_prompt_shield_cost(&usage, cost_tier, price.map(decimal)).expect("the cost resolves");
    if expected.is_empty() {
        assert!(cost.is_none());
    } else {
        assert_eq!(cost, Some(decimal(expected)));
    }
}

#[test]
fn azure_prompt_shield_guardrail_cost_treats_a_missing_count_as_zero_records() {
    let empty_usage = BTreeMap::new();
    let cost = azure_prompt_shield_cost(&empty_usage, None, Some(decimal("5")))
        .expect("the cost resolves");
    assert_eq!(cost, Some(Decimal::ZERO));
}

#[rstest]
#[case::null_information("null", "0")]
#[case::single_object_not_in_a_list(r#"{"guardrail_cost": 5}"#, "5")]
#[case::positive_cost_is_billed(r#"[{"guardrail_cost": 2.5}]"#, "2.5")]
#[case::zero_cost_contributes_nothing(r#"[{"guardrail_cost": 0}]"#, "0")]
#[case::negative_cost_contributes_nothing(r#"[{"guardrail_cost": -5}]"#, "0")]
#[case::null_in_spend_retains_the_default_billing(
    r#"[{"guardrail_cost": 3, "guardrail_cost_in_spend": null}]"#,
    "3"
)]
#[case::false_in_spend_opts_out(
    r#"[{"guardrail_cost": 3, "guardrail_cost_in_spend": false}]"#,
    "0"
)]
#[case::missing_in_spend_defaults_true(r#"[{"guardrail_cost": 4}]"#, "4")]
#[case::nonfinite_string_cost_contributes_nothing(r#"[{"guardrail_cost": "inf"}]"#, "0")]
#[case::boolean_cost_coerces_like_pydantic(r#"[{"guardrail_cost": true}]"#, "1")]
#[case::numeric_string_in_spend_coerces(
    r#"[{"guardrail_cost": 2, "guardrail_cost_in_spend": "yes"}]"#,
    "2"
)]
#[case::a_malformed_shape_zeros_only_that_entry(r#"[{"guardrail_cost": 5}, [1, 2, 3]]"#, "5")]
#[case::an_uncoercible_in_spend_zeros_the_whole_entry(
    r#"[{"guardrail_cost": 5, "guardrail_cost_in_spend": "maybe"}]"#,
    "0"
)]
#[case::a_malformed_entry_keeps_a_valid_sibling(
    r#"[{"guardrail_cost": 5, "guardrail_cost_in_spend": "maybe"}, {"guardrail_cost": 7}]"#,
    "7"
)]
#[case::a_forged_by_unit_map_does_not_change_the_scalar_cost(
    r#"[{"guardrail_cost": 4, "guardrail_cost_by_unit": {"contentPolicyUnits": -1}}]"#,
    "4"
)]
fn guardrail_information_cost_matches_the_source_behavior(
    #[case] guardrail_information: &str,
    #[case] expected: &str,
) {
    let cost =
        guardrail_information_cost(&information(guardrail_information)).expect("cost resolves");
    assert_eq!(cost, decimal(expected));
}

#[test]
fn guardrail_information_cost_propagates_a_sum_overflow_across_entries() {
    let payload = r#"[{"guardrail_cost": "79228162514264337593543950335"}, {"guardrail_cost": 1}]"#;
    let error =
        guardrail_information_cost(&information(payload)).expect_err("the sum overflows Decimal");
    assert!(matches!(error, GuardrailPricingError::CostOverflow));
}

#[rstest]
#[case::missing_entry("{}", None)]
#[case::known_and_null_counters(
    r#"{"guardrail_cost_by_unit": {"a": 1.5, "b": null}}"#,
    Some(vec![("a", Some("1.5")), ("b", None)])
)]
#[case::explicit_null_map(r#"{"guardrail_cost_by_unit": null}"#, None)]
#[case::false_in_spend_suppresses_a_valid_map(
    r#"{"guardrail_cost_by_unit": {"a": 1}, "guardrail_cost_in_spend": false}"#,
    None
)]
#[case::a_negative_counter_invalidates_the_whole_entry(
    r#"{"guardrail_cost_by_unit": {"a": 1, "b": -1}}"#,
    None
)]
#[case::a_nonfinite_counter_invalidates_the_whole_entry(
    r#"{"guardrail_cost_by_unit": {"a": "nan"}}"#,
    None
)]
#[case::an_uncoercible_shape_is_malformed("[1, 2]", None)]
#[case::a_forged_scalar_field_does_not_change_the_rollup(
    r#"{"guardrail_cost_by_unit": {"a": 1}, "guardrail_cost": "not-a-number"}"#,
    Some(vec![("a", Some("1"))])
)]
fn billed_guardrail_cost_by_unit_matches_the_source_behavior(
    #[case] raw: &str,
    #[case] expected: Option<Vec<(&str, Option<&str>)>>,
) {
    let costs = billed_guardrail_cost_by_unit(&entry(raw)).expect("rollup validation resolves");
    let expected_costs = expected.map(|pairs| {
        pairs
            .into_iter()
            .map(|(key, rate)| (key.to_owned(), rate.map(decimal)))
            .collect::<BTreeMap<_, _>>()
    });
    assert_eq!(costs, expected_costs);
}
