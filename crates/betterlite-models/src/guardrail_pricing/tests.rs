#![cfg(test)]

use super::entries::{
    GuardrailInformation, GuardrailInformationEntry, billed_guardrail_cost_by_unit,
    guardrail_information_cost,
};
use super::{
    GuardrailPricingError, azure_prompt_shield_cost, bedrock_guardrail_cost_by_unit,
    guardrail_cost_total,
};
use crate::catalog::{ModelCatalog, UnknownFields};
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

fn catalog(prices: &[(&str, &str)]) -> ModelCatalog {
    let entries = prices
        .iter()
        .map(|(name, cost)| {
            let mut entry = serde_json::json!({
                "litellm_provider": "bedrock",
                "mode": "guardrail",
            });
            if *cost != "__missing" {
                entry["guardrail_cost_per_unit"] =
                    serde_json::from_str::<serde_json::Value>(cost).expect("price map parses");
            }
            ((*name).to_owned(), entry)
        })
        .collect::<serde_json::Map<String, serde_json::Value>>();
    let source = serde_json::Value::Object(entries).to_string();
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("fixture catalog parses")
}

fn information(text: &str) -> GuardrailInformation {
    serde_json::from_str(text).expect("a fixture guardrail_information parses")
}

fn entry(text: &str) -> GuardrailInformationEntry {
    serde_json::from_str(text).expect("a fixture guardrail_information entry parses")
}

#[test]
fn bedrock_guardrail_cost_by_unit_prefers_the_regional_key_when_the_region_is_truthy() {
    let catalog = catalog(&[
        (
            "bedrock/us-east-1/guardrails",
            r#"{"contentPolicyUnits":2}"#,
        ),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":9}"#),
    ]);
    let costs = bedrock_guardrail_cost_by_unit(
        &catalog,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 3)]),
    )
    .expect("pricing resolves")
    .expect("regional map found");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("6")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_falls_through_a_regional_entry_without_unit_prices() {
    let catalog = catalog(&[
        ("bedrock/us-east-1/guardrails", "__missing"),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":5}"#),
    ]);
    let costs = bedrock_guardrail_cost_by_unit(
        &catalog,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 2)]),
    )
    .expect("pricing resolves")
    .expect("global map found");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("10")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_falls_through_a_regional_entry_with_null_unit_prices() {
    let catalog = catalog(&[
        ("bedrock/us-east-1/guardrails", "null"),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":5}"#),
    ]);
    let costs = bedrock_guardrail_cost_by_unit(
        &catalog,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 2)]),
    )
    .expect("pricing resolves")
    .expect("global map found");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("10")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_skips_a_priced_regional_key_for_an_empty_region() {
    let catalog = catalog(&[
        ("bedrock//guardrails", r#"{"contentPolicyUnits":9}"#),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":5}"#),
    ]);
    let costs =
        bedrock_guardrail_cost_by_unit(&catalog, Some(""), &usage(&[("contentPolicyUnits", 1)]))
            .expect("pricing resolves")
            .expect("global map found");
    assert_eq!(costs["contentPolicyUnits"], Some(decimal("5")));
}

#[test]
fn bedrock_guardrail_cost_by_unit_lets_a_valid_empty_regional_map_block_global_fallback() {
    let catalog = catalog(&[
        ("bedrock/us-east-1/guardrails", "{}"),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":5}"#),
    ]);
    let costs = bedrock_guardrail_cost_by_unit(
        &catalog,
        Some("us-east-1"),
        &usage(&[("contentPolicyUnits", 2)]),
    )
    .expect("pricing resolves")
    .expect("empty regional map found");
    assert_eq!(costs["contentPolicyUnits"], None);
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
#[case::a_list_shaped_entry_bills_nothing(r#"[{"guardrail_cost": 2}, [5], [5, true]]"#, "2")]
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
#[case::a_list_shaped_entry_has_no_rollup(r#"[null, null, {"a": 1}]"#, None)]
#[case::a_bare_number_is_not_a_counter_map(r#"{"guardrail_cost_by_unit": 2.5}"#, None)]
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
