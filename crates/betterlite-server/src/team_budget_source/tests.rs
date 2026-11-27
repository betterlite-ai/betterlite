#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::{TeamMemberBudgetSource, member_budget_source, team_default_budget_id};

fn fields(value: JsonData) -> BTreeMap<String, JsonData> {
    let mut metadata = BTreeMap::new();
    metadata.insert("team_member_budget_id".to_owned(), value);
    metadata
}

#[test]
fn default_id_string() {
    let metadata = fields(JsonData::String("budget-a".to_owned()));
    assert_eq!(team_default_budget_id(Some(&metadata)), Some("budget-a"));
}

#[test]
fn default_id_empty_string() {
    let metadata = fields(JsonData::String(String::new()));
    assert_eq!(team_default_budget_id(Some(&metadata)), Some(""));
}

#[rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::integer("7")]
#[case::float("1.5")]
#[case::array(r#"["b"]"#)]
#[case::object(r#"{"budget_id": "b"}"#)]
fn default_id_wrong_type(#[case] value: &str) {
    let metadata = fields(serde_json::from_str::<JsonData>(value).unwrap());
    assert_eq!(team_default_budget_id(Some(&metadata)), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty_map(Some("{}"))]
#[case::other_keys(Some(r#"{"team_name": "core"}"#))]
fn default_id_missing(#[case] source: Option<&str>) {
    let metadata: Option<BTreeMap<String, JsonData>> =
        source.map(|text| serde_json::from_str::<BTreeMap<String, JsonData>>(text).unwrap());
    assert_eq!(team_default_budget_id(metadata.as_ref()), None);
}

#[rstest]
#[case::unlike_the_default(Some("member-a"), Some("team-default"))]
#[case::member_id_without_a_default(Some("member-a"), None)]
fn source_custom(#[case] budget_id: Option<&str>, #[case] default_id: Option<&str>) {
    assert_eq!(
        member_budget_source(budget_id, default_id),
        TeamMemberBudgetSource::Custom
    );
}

#[rstest]
#[case::equal_to_the_default(Some("member-a"), Some("member-a"))]
#[case::no_member_id(None, Some("team-default"))]
#[case::both_empty(Some(""), Some(""))]
fn source_team_default(#[case] budget_id: Option<&str>, #[case] default_id: Option<&str>) {
    assert_eq!(
        member_budget_source(budget_id, default_id),
        TeamMemberBudgetSource::TeamDefault
    );
}

#[test]
fn source_no_budget() {
    assert_eq!(
        member_budget_source(None, None),
        TeamMemberBudgetSource::NoBudget
    );
}

#[rstest]
#[case::team_default(TeamMemberBudgetSource::TeamDefault, "team_default")]
#[case::custom(TeamMemberBudgetSource::Custom, "custom")]
#[case::no_budget(TeamMemberBudgetSource::NoBudget, "none")]
fn wire_source_values(#[case] source: TeamMemberBudgetSource, #[case] text: &str) {
    let serialized = serde_json::to_string(&source).unwrap();
    assert_eq!(serialized, format!("\"{text}\""));
    assert_eq!(
        serde_json::from_str::<TeamMemberBudgetSource>(&serialized).unwrap(),
        source
    );
}

#[rstest]
#[case::member("member")]
#[case::rust_none("None")]
#[case::empty("")]
fn wire_source_refusals(#[case] text: &str) {
    serde_json::from_str::<TeamMemberBudgetSource>(&format!("\"{text}\"")).unwrap_err();
}
