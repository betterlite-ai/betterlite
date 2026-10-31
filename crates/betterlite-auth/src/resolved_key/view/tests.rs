#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::{VerificationTokenView, normalize_fields};

fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("invalid JSON fixture: {error}"))
}

fn object(text: &str) -> BTreeMap<String, JsonData> {
    match parse(text) {
        JsonData::Object(fields) => fields,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => panic!("fixture must be a JSON object"),
    }
}

fn field(view: &BTreeMap<String, JsonData>, name: &str) -> JsonData {
    view.get(name).cloned().unwrap_or(JsonData::Null)
}

fn view_from(text: &str) -> BTreeMap<String, JsonData> {
    object(
        &serde_json::to_string(&VerificationTokenView::construct(object(text)).unwrap()).unwrap(),
    )
}

#[rstest]
#[case::view_field_team_spend("team_spend", "12.5")]
#[case::view_field_team_alias("team_alias", "\"prod\"")]
#[case::view_field_team_tpm_limit("team_tpm_limit", "10")]
#[case::view_field_team_rpm_limit("team_rpm_limit", "10")]
#[case::view_field_team_tpd_limit("team_tpd_limit", "10")]
#[case::view_field_team_max_budget("team_max_budget", "-5.0")]
#[case::view_field_team_soft_budget("team_soft_budget", "1.5")]
#[case::view_field_team_model_max_budget("team_model_max_budget", "{\"gpt-4\": 1.0}")]
#[case::view_field_soft_budget("soft_budget", "2.5")]
#[case::view_field_team_model_aliases("team_model_aliases", "{\"a\": \"b\"}")]
#[case::view_field_team_member(
    "team_member",
    "{\"user_id\": \"u1\", \"user_email\": null, \"role\": \"admin\"}"
)]
#[case::view_field_team_metadata("team_metadata", "{\"k\": \"v\"}")]
#[case::view_field_team_object_permission_id("team_object_permission_id", "\"perm-1\"")]
#[case::view_field_team_member_spend("team_member_spend", "3.5")]
#[case::view_field_team_member_tpm_limit("team_member_tpm_limit", "1")]
#[case::view_field_team_member_rpm_limit("team_member_rpm_limit", "1")]
#[case::view_field_end_user_id("end_user_id", "\"end-1\"")]
#[case::view_field_end_user_tpm_limit("end_user_tpm_limit", "1")]
#[case::view_field_end_user_rpm_limit("end_user_rpm_limit", "1")]
#[case::view_field_end_user_tpd_limit("end_user_tpd_limit", "1")]
#[case::view_field_end_user_max_budget("end_user_max_budget", "4.5")]
#[case::view_field_end_user_model_max_budget("end_user_model_max_budget", "{\"a\": 1.0}")]
#[case::view_field_organization_alias("organization_alias", "\"org\"")]
#[case::view_field_organization_max_budget("organization_max_budget", "6.5")]
#[case::view_field_organization_tpm_limit("organization_tpm_limit", "1")]
#[case::view_field_organization_rpm_limit("organization_rpm_limit", "1")]
#[case::view_field_organization_metadata("organization_metadata", "{\"k\": \"v\"}")]
#[case::view_field_project_alias("project_alias", "\"proj\"")]
#[case::view_field_project_metadata("project_metadata", "{\"k\": \"v\"}")]
#[case::view_field_last_refreshed_at("last_refreshed_at", "100.0")]
fn view_field_cases(#[case] field_name: &str, #[case] value_text: &str) {
    let value = parse(value_text);
    assert_eq!(field(&view_from("{}"), field_name), JsonData::Null);
    assert_eq!(
        field(
            &view_from(&format!("{{\"{field_name}\": null}}")),
            field_name
        ),
        JsonData::Null
    );
    let input = format!("{{\"{field_name}\": {value_text}}}");
    assert_eq!(field(&view_from(&input), field_name), value);
}

#[test]
fn view_field_team_blocked() {
    assert_eq!(field(&view_from("{}"), "team_blocked"), parse("false"));
    assert_eq!(
        field(&view_from("{\"team_blocked\": true}"), "team_blocked"),
        parse("true")
    );
}

#[test]
fn view_field_team_models() {
    assert_eq!(field(&view_from("{}"), "team_models"), parse("[]"));
}

#[test]
fn generic_team_models_preserved() {
    let models = "[\"gpt-4\", {\"model\": \"claude\"}, 1]";
    let input = format!("{{\"team_models\": {models}}}");
    assert_eq!(field(&view_from(&input), "team_models"), parse(models));
}

#[rstest]
#[case::current_absent(
    "{\"litellm_budget_table_team_rpm_limit\": 10}",
    "team_rpm_limit",
    "10"
)]
#[case::current_null(
    "{\"litellm_budget_table_team_alias\": \"x\", \"team_alias\": null}",
    "team_alias",
    "\"x\""
)]
#[case::model_max_budget_empty_dict(
    "{\"litellm_budget_table_model_max_budget\": {\"gpt-4\": 1.0}, \"model_max_budget\": {}}",
    "model_max_budget",
    "{\"gpt-4\": 1.0}"
)]
fn flattened_budget_null_and_empty_dict_fill(
    #[case] fields: &str,
    #[case] field_name: &str,
    #[case] expected: &str,
) {
    assert_eq!(field(&view_from(fields), field_name), parse(expected));
}

#[rstest]
#[case::zero_wins(
    "{\"litellm_budget_table_team_rpm_limit\": 10, \"team_rpm_limit\": 0}",
    "team_rpm_limit",
    "0"
)]
#[case::false_wins(
    "{\"litellm_budget_table_team_blocked\": true, \"team_blocked\": false}",
    "team_blocked",
    "false"
)]
#[case::empty_string_wins(
    "{\"litellm_budget_table_team_alias\": \"x\", \"team_alias\": \"\"}",
    "team_alias",
    "\"\""
)]
#[case::nonempty_dict_wins(
    "{\"litellm_budget_table_model_max_budget\": {\"gpt-4\": 1.0}, \"model_max_budget\": {\"claude\": 2.0}}",
    "model_max_budget",
    "{\"claude\": 2.0}"
)]
fn flattened_budget_zero_false_empty_string_win(
    #[case] fields: &str,
    #[case] field_name: &str,
    #[case] expected: &str,
) {
    assert_eq!(field(&view_from(fields), field_name), parse(expected));
}

#[test]
fn json_string_not_empty_dict() {
    let fields = "{\"litellm_budget_table_team_model_aliases\": {\"a\": \"b\"}, \"team_model_aliases\": \"{}\"}";
    let outcome = VerificationTokenView::construct(object(fields));
    outcome.unwrap_err();
}

#[test]
fn budget_prefix_removed_everywhere() {
    let fields = "{\"litellm_budget_table_litellm_budget_table_spend\": 99}";
    assert_eq!(field(&view_from(fields), "spend"), parse("99.0"));
}

#[test]
fn budget_added_values_not_revisited() {
    let mut fields = object("{\"litellm_budget_table_end_user_id\": true}");
    normalize_fields(&mut fields);
    assert_eq!(field(&fields, "end_user_id"), parse("true"));

    let fresh = object("{\"litellm_budget_table_end_user_id\": true}");
    let outcome = VerificationTokenView::construct(fresh);
    outcome.unwrap_err();
}

#[rstest]
#[case::positive_int("42", "\"42\"")]
#[case::negative_int("-7", "\"-7\"")]
#[case::true_value("true", "\"True\"")]
#[case::false_value("false", "\"False\"")]
fn integer_and_bool_end_user_id(#[case] input: &str, #[case] expected: &str) {
    let fields = format!("{{\"end_user_id\": {input}}}");
    assert_eq!(field(&view_from(&fields), "end_user_id"), parse(expected));
}

#[test]
fn organization_id_overrides_org_id() {
    let fields = "{\"org_id\": \"explicit\", \"organization_id\": \"override\"}";
    assert_eq!(field(&view_from(fields), "org_id"), parse("\"override\""));
}

#[test]
fn organization_null_does_not_override() {
    let fields = "{\"org_id\": \"explicit\", \"organization_id\": null}";
    assert_eq!(field(&view_from(fields), "org_id"), parse("\"explicit\""));
}
