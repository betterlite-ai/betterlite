#![cfg(test)]
use super::{
    _get_value as get_value, _to_float as to_float, _to_int as to_int, BudgetCounter,
    CounterEntityType,
};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::HashMap;
#[test]
fn structure_has_all_counter_fields() {
    let counter = BudgetCounter {
        counter_key: "spend:key:test".to_owned(),
        max_budget: Decimal::from(10_i32),
        fallback_spend: Decimal::from(1_i32),
        entity_type: CounterEntityType::Key,
        entity_id: "test".to_owned(),
        source_cache_key: Some("cache".to_owned()),
        spend_log_entity_id: Some("log".to_owned()),
        window_duration: Some("1h".to_owned()),
        window_start: None,
    };
    assert_eq!(counter.entity_id, "test");
}
#[rstest]
#[case("Key", CounterEntityType::Key, "key")]
#[case("Team", CounterEntityType::Team, "team")]
#[case("TeamMember", CounterEntityType::TeamMember, "team_member")]
#[case("User", CounterEntityType::User, "user")]
#[case("EndUser", CounterEntityType::EndUser, "end_user")]
#[case("Tag", CounterEntityType::Tag, "tag")]
#[case(
    "Model access group",
    CounterEntityType::ModelAccessGroup,
    "model_access_group"
)]
#[case("Organization", CounterEntityType::Organization, "organization")]
#[case("Project", CounterEntityType::Project, "project")]
fn entity_mapping_handles_all_names(
    #[case] label: &str,
    #[case] kind: CounterEntityType,
    #[case] wire: &str,
) {
    assert_eq!(CounterEntityType::from_litellm_name(label), Some(kind));
    assert_eq!(kind.wire_value(), wire);
}
#[rstest]
#[case(2_i32, Decimal::from(2_i32))]
#[case(2.5_f64, Decimal::try_from(2.5_f64).unwrap())]
#[case(" 2.5\n", Decimal::new(25, 1))]
#[case("1_000.25", Decimal::new(100_025, 2))]
fn to_float_accepts_values<T>(#[case] value: T, #[case] expected: Decimal)
where
    T: super::FloatInput,
{
    assert_eq!(to_float(value), Some(expected));
    assert_eq!(to_float(&b" 3.5\n"[..]), Some(Decimal::new(35, 1)));
    assert_eq!(to_float(true), Some(Decimal::ONE));
    assert_eq!(to_float("abc"), None);
    assert_eq!(to_float(Vec::from([0xff, 0xfe])), None);
}
#[rstest]
#[case(2_i32, 2_i64)]
#[case(2.9_f64, 2_i64)]
#[case(" 1_000\n", 1_000_i64)]
fn to_int_accepts_values<T>(#[case] value: T, #[case] expected: i64)
where
    T: super::IntInput,
{
    assert!(matches!(to_int(value), Some(actual) if actual == expected));
    assert_eq!(to_int("abc"), None);
    assert_eq!(to_int(Vec::from([0xff, 0xfe])), None);
    assert_eq!(to_int(u128::MAX), None);
    assert_eq!(to_int(f64::NAN), None);
}
struct Object(String);
impl super::ValueLookup for Object {
    fn value(&self, key: &str) -> Option<&String> {
        (key == "spend").then_some(&self.0)
    }
}
#[test]
fn get_value_reads_mapping_and_object_without_conversion() {
    let values = HashMap::from([(String::from("spend"), String::from("2.5"))]);
    assert_eq!(get_value(&values, "spend"), Some(&String::from("2.5")));
    assert_eq!(get_value(&values, "missing"), None);
    assert_eq!(
        get_value(&Object(String::from("3")), "spend"),
        Some(&String::from("3"))
    );
}
