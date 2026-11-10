use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;
use rust_decimal::Decimal;

use super::{declaration, model_info};
use crate::ptu_pricing::PtuModelInfo;
use crate::ptu_pricing::pricing_fields::{
    CUSTOM_PRICING_FIELDS, SEARCH_CONTEXT_QUERY_FIELD, TIERED_PRICING_FIELD, ZEROED_RATE_FIELDS,
};
use crate::ptu_pricing::zeroed::{PtuZero, PtuZeroedPricing, zeroed_ptu_pricing};
use crate::tiered_pricing::Scalar;

const STANDING_SET_SIZE: usize = 12;

type ModelInfoMap = BTreeMap<String, Scalar>;

fn declared(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn complete_info() -> ModelInfoMap {
    model_info(&declaration(
        r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1""#,
    ))
}

fn zeroed(names: &[&str]) -> Option<PtuZeroedPricing> {
    let info = complete_info();
    zeroed_ptu_pricing(&PtuModelInfo::from(&info), &declared(names), true)
}

#[test]
fn zeroed_pricing_requires_chargeable_terms() {
    let info = model_info(r#"{"ptu_count":1}"#);
    assert_eq!(
        zeroed_ptu_pricing(&PtuModelInfo::from(&info), &BTreeSet::new(), true),
        None
    );
}

#[test]
fn zeroed_pricing_requires_the_feature_flag() {
    let info = complete_info();
    assert_eq!(
        zeroed_ptu_pricing(&PtuModelInfo::from(&info), &BTreeSet::new(), false),
        None
    );
}

#[test]
fn zeroed_pricing_is_the_standing_set_without_declarations() {
    assert_eq!(zeroed(&[]).unwrap().entries.len(), STANDING_SET_SIZE);
}

#[rstest]
fn zeroed_pricing_zeroes_every_standing_rate_field() {
    let pricing = zeroed(&[]).unwrap();
    for field in ZEROED_RATE_FIELDS {
        assert_eq!(
            pricing.entries.get(field),
            Some(&PtuZero::Rate(Decimal::ZERO)),
            "{field}"
        );
    }
}

#[test]
fn zeroed_pricing_empties_the_tier_table() {
    let pricing = zeroed(&[]).unwrap();
    assert_eq!(
        pricing.entries.get(TIERED_PRICING_FIELD),
        Some(&PtuZero::EmptyTiers)
    );
}

#[test]
fn zeroed_pricing_writes_every_search_context_size() {
    let pricing = zeroed(&[]).unwrap();
    let Some(PtuZero::SearchContext(table)) = pricing.entries.get(SEARCH_CONTEXT_QUERY_FIELD)
    else {
        panic!("the search context table is always written");
    };
    assert_eq!(table.search_context_size_low, Some(Decimal::ZERO));
    assert_eq!(table.search_context_size_medium, Some(Decimal::ZERO));
    assert_eq!(table.search_context_size_high, Some(Decimal::ZERO));
}

#[test]
fn zeroed_pricing_keeps_the_search_context_table_over_a_declared_zero() {
    let pricing = zeroed(&[SEARCH_CONTEXT_QUERY_FIELD]).unwrap();
    assert!(matches!(
        pricing.entries.get(SEARCH_CONTEXT_QUERY_FIELD),
        Some(PtuZero::SearchContext(_))
    ));
}

#[rstest]
#[case::per_second("input_cost_per_second")]
#[case::per_image("output_cost_per_image")]
#[case::per_character("input_cost_per_character")]
#[case::grounding("google_maps_grounding_cost_per_query")]
fn zeroed_pricing_zeroes_a_declared_custom_rate(#[case] field: &str) {
    let pricing = zeroed(&[field]).unwrap();
    assert_eq!(
        pricing.entries.get(field),
        Some(&PtuZero::Rate(Decimal::ZERO))
    );
}

#[rstest]
#[case::vector_size("output_vector_size")]
#[case::reservation_rate("cost_per_ptu_per_hour")]
#[case::unrelated_setting("tiered_pricing_override")]
fn zeroed_pricing_ignores_a_declared_non_cost_field(#[case] field: &str) {
    let pricing = zeroed(&[field]).unwrap();
    assert_eq!(pricing.entries.get(field), None);
}

#[rstest]
fn zeroed_pricing_covers_every_custom_pricing_field() {
    let all = CUSTOM_PRICING_FIELDS.to_vec();
    let pricing = zeroed(&all).unwrap();
    for field in CUSTOM_PRICING_FIELDS {
        assert!(pricing.entries.contains_key(field), "{field}");
        if field != SEARCH_CONTEXT_QUERY_FIELD {
            assert_eq!(
                pricing.entries.get(field),
                Some(&PtuZero::Rate(Decimal::ZERO)),
                "{field}"
            );
        }
    }
    assert_eq!(
        pricing.entries.len(),
        CUSTOM_PRICING_FIELDS.len().saturating_add(1)
    );
}

#[rstest]
fn custom_pricing_inventory_matches_the_pinned_source() {
    let fixture: Vec<&str> = include_str!("../fixtures/custom_pricing_fields.txt")
        .lines()
        .collect();
    assert_eq!(CUSTOM_PRICING_FIELDS.to_vec(), fixture);
}

#[rstest]
fn zeroed_rate_fields_are_part_of_the_custom_inventory() {
    for field in ZEROED_RATE_FIELDS {
        assert!(CUSTOM_PRICING_FIELDS.contains(&field), "{field}");
    }
}
