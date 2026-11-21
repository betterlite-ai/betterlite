#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::priority_from_key_dict;

fn metadata(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

#[test]
fn a_team_priority_wins_over_the_key_priority() {
    let team = metadata(&[("priority", text("team"))]);
    let key = metadata(&[("priority", text("key"))]);
    assert_eq!(priority_from_key_dict(Some(&team), &key), Some("team"));
}

#[test]
fn the_key_priority_answers_when_the_team_has_none() {
    let team = metadata(&[("team_id", text("acme"))]);
    let key = metadata(&[("priority", text("key"))]);
    assert_eq!(priority_from_key_dict(Some(&team), &key), Some("key"));
}

#[test]
fn an_absent_team_leaves_the_key_priority() {
    let key = metadata(&[("priority", text("key"))]);
    assert_eq!(priority_from_key_dict(None, &key), Some("key"));
}

#[test]
fn a_null_team_priority_falls_back_to_the_key() {
    let team = metadata(&[("priority", JsonData::Null)]);
    let key = metadata(&[("priority", text("key"))]);
    assert_eq!(priority_from_key_dict(Some(&team), &key), Some("key"));
}

#[test]
fn a_null_key_priority_names_no_pool() {
    let key = metadata(&[("priority", JsonData::Null)]);
    assert_eq!(priority_from_key_dict(None, &key), None);
}

#[test]
fn no_priority_anywhere_names_no_pool() {
    let empty = BTreeMap::new();
    assert_eq!(priority_from_key_dict(None, &empty), None);
    let team = metadata(&[("team_id", text("acme"))]);
    assert_eq!(priority_from_key_dict(Some(&team), &empty), None);
}

#[rstest]
#[case::bool(JsonData::Bool(true))]
#[case::number(JsonData::Number(serde_json::Number::from(3_u32)))]
#[case::array(JsonData::Array(Vec::new()))]
#[case::object(JsonData::Object(BTreeMap::new()))]
fn a_non_text_team_priority_shadows_the_key(#[case] value: JsonData) {
    let team = metadata(&[("priority", value)]);
    let key = metadata(&[("priority", text("key"))]);
    assert_eq!(priority_from_key_dict(Some(&team), &key), None);
}

#[rstest]
#[case::bool(JsonData::Bool(true))]
#[case::number(JsonData::Number(serde_json::Number::from(3_u32)))]
#[case::array(JsonData::Array(Vec::new()))]
#[case::object(JsonData::Object(BTreeMap::new()))]
fn a_non_text_key_priority_names_no_pool(#[case] value: JsonData) {
    let key = metadata(&[("priority", value)]);
    assert_eq!(priority_from_key_dict(None, &key), None);
}
