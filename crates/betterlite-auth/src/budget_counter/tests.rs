#![cfg(test)]
use super::{BudgetCounter, CounterEntityType};
use rstest::rstest;
use rust_decimal::Decimal;
#[test]
fn structure_has_all_counter_fields() {
    let _counter = BudgetCounter {
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
}
#[rstest]
#[case("Key", Some("key"))]
#[case("Team", Some("team"))]
#[case("TeamMember", Some("team_member"))]
#[case("User", Some("user"))]
#[case("EndUser", Some("end_user"))]
#[case("Tag", Some("tag"))]
#[case("Model access group", Some("model_access_group"))]
#[case("Organization", Some("organization"))]
#[case("Project", Some("project"))]
#[case("model_access_group", None)]
#[case("ModelAccessGroup", None)]
fn entity_mapping_handles_all_names(#[case] label: &str, #[case] wire: Option<&str>) {
    let kind = CounterEntityType::from_litellm_name(label);
    assert_eq!(kind.as_ref().map(CounterEntityType::wire_value), wire);
}
