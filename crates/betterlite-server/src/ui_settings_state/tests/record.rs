use betterlite_config::sections::team_admin_fields::{
    MEMBER_KEY_BUDGETS_PERMISSION, PROJECTS_PERMISSION, SUPPORTED_TEAM_FIELDS,
};
use rstest::rstest;

use super::support::{json, settings};
use crate::ui_settings_state::record::{UiSettingErrorKind, UiSettings, UiSettingsError};
use crate::ui_settings_state::schema::ui_settings_schema;

const DEFAULT_DUMP: &str = r#"{
    "disable_model_add_for_internal_users": false,
    "disable_team_admin_delete_team_user": false,
    "enabled_ui_pages_internal_users": null,
    "require_auth_for_public_ai_hub": false,
    "allow_public_health_readiness_details": false,
    "forward_client_headers_to_llm_api": false,
    "forward_llm_provider_auth_headers": false,
    "disable_agents_for_internal_users": false,
    "allow_agents_for_team_admins": false,
    "disable_vector_stores_for_internal_users": false,
    "allow_vector_stores_for_team_admins": false,
    "scope_user_search_to_org": false,
    "disable_custom_api_keys": false,
    "disable_key_generate_for_org_admin": false,
    "enable_chat_ui": false,
    "team_admin_editable_team_fields": []
}"#;

#[test]
fn record_dumps_every_field_with_its_model_default() {
    let record = UiSettings::validate(&settings("{}")).expect("the empty settings validate");
    assert_eq!(record.model_dump(), settings(DEFAULT_DUMP));
    assert_eq!(record.model_dump_exclude_unset(), settings("{}"));
}

#[rstest]
#[case::flag_true("true", "true")]
#[case::flag_false("false", "false")]
#[case::integer_one("1", "true")]
#[case::integer_zero("0", "false")]
#[case::float_one("1.0", "true")]
#[case::float_zero("0.0", "false")]
#[case::negative_zero("-0.0", "false")]
#[case::exponent_one("1e0", "true")]
#[case::text_true(r#""true""#, "true")]
#[case::text_upper(r#""TRUE""#, "true")]
#[case::text_yes(r#""yes""#, "true")]
#[case::text_off(r#""off""#, "false")]
#[case::text_t(r#""t""#, "true")]
#[case::text_n(r#""n""#, "false")]
#[case::text_one(r#""1""#, "true")]
#[case::text_zero(r#""0""#, "false")]
fn record_coerces_booleans_like_the_pydantic_model(#[case] value: &str, #[case] expected: &str) {
    let input = format!(r#"{{"enable_chat_ui": {value}}}"#);
    let record = UiSettings::validate(&settings(&input)).expect("the value coerces");
    assert_eq!(
        record.model_dump_exclude_unset(),
        settings(&format!(r#"{{"enable_chat_ui": {expected}}}"#))
    );
}

#[rstest]
#[case::text_outside_the_set(r#""maybe""#, UiSettingErrorKind::BoolParsing)]
#[case::blank_text(r#"" ""#, UiSettingErrorKind::BoolParsing)]
#[case::integer_two("2", UiSettingErrorKind::BoolParsing)]
#[case::float_two("2.0", UiSettingErrorKind::BoolParsing)]
#[case::negative_float("-1.0", UiSettingErrorKind::BoolParsing)]
#[case::float_fraction("1.5", UiSettingErrorKind::BoolType)]
#[case::tiny_float("1e-21", UiSettingErrorKind::BoolType)]
#[case::huge_float("1e21", UiSettingErrorKind::BoolType)]
#[case::huge_integer("9223372036854775808", UiSettingErrorKind::BoolType)]
#[case::huge_unsigned_integer("18446744073709551615", UiSettingErrorKind::BoolType)]
#[case::null("null", UiSettingErrorKind::BoolType)]
#[case::array("[]", UiSettingErrorKind::BoolType)]
#[case::object("{}", UiSettingErrorKind::BoolType)]
fn record_rejects_booleans_the_pydantic_model_rejects(
    #[case] value: &str,
    #[case] kind: UiSettingErrorKind,
) {
    let input = format!(r#"{{"enable_chat_ui": {value}}}"#);
    let error = UiSettings::validate(&settings(&input)).expect_err("the value is rejected");
    assert_eq!(
        error,
        UiSettingsError {
            kind,
            field: "enable_chat_ui",
            index: None,
        }
    );
}

#[test]
fn record_accepts_null_and_string_lists_for_the_page_setting() {
    let empty = UiSettings::validate(&settings(r#"{"enabled_ui_pages_internal_users": null}"#))
        .expect("null is the field default");
    assert_eq!(
        empty
            .model_dump_exclude_unset()
            .get("enabled_ui_pages_internal_users"),
        Some(&json("null"))
    );
    let pages = UiSettings::validate(&settings(
        r#"{"enabled_ui_pages_internal_users": ["chat", "models"]}"#,
    ))
    .expect("the pages validate");
    assert_eq!(
        pages.model_dump().get("enabled_ui_pages_internal_users"),
        Some(&json(r#"["chat", "models"]"#))
    );
}

#[rstest]
#[case::text(r#""pages""#, UiSettingErrorKind::ListType, None)]
#[case::number("3", UiSettingErrorKind::ListType, None)]
#[case::flag("true", UiSettingErrorKind::ListType, None)]
#[case::object("{}", UiSettingErrorKind::ListType, None)]
#[case::number_item(r#"["chat", 1]"#, UiSettingErrorKind::StringType, Some(1))]
#[case::null_item("[null]", UiSettingErrorKind::StringType, Some(0))]
#[case::nested_item("[[]]", UiSettingErrorKind::StringType, Some(0))]
fn record_rejects_page_settings_the_pydantic_model_rejects(
    #[case] value: &str,
    #[case] kind: UiSettingErrorKind,
    #[case] index: Option<usize>,
) {
    let input = format!(r#"{{"enabled_ui_pages_internal_users": {value}}}"#);
    let error = UiSettings::validate(&settings(&input)).expect_err("the value is rejected");
    assert_eq!(
        error,
        UiSettingsError {
            kind,
            field: "enabled_ui_pages_internal_users",
            index,
        }
    );
}

#[test]
fn record_accepts_string_lists_for_the_team_fields() {
    let record = UiSettings::validate(&settings(
        r#"{"team_admin_editable_team_fields": ["max_budget", "projects"]}"#,
    ))
    .expect("the team fields validate");
    assert_eq!(
        record.model_dump().get("team_admin_editable_team_fields"),
        Some(&json(r#"["max_budget", "projects"]"#))
    );
    let default = UiSettings::validate(&settings("{}")).expect("the defaults validate");
    assert_eq!(
        default.model_dump().get("team_admin_editable_team_fields"),
        Some(&json("[]"))
    );
}

#[rstest]
#[case::text(r#""max_budget""#, UiSettingErrorKind::SequenceStr, None)]
#[case::null("null", UiSettingErrorKind::IsInstanceOf, None)]
#[case::number("3", UiSettingErrorKind::IsInstanceOf, None)]
#[case::flag("true", UiSettingErrorKind::IsInstanceOf, None)]
#[case::object("{}", UiSettingErrorKind::IsInstanceOf, None)]
#[case::number_item(r#"["max_budget", 1]"#, UiSettingErrorKind::StringType, Some(1))]
fn record_rejects_team_fields_the_pydantic_model_rejects(
    #[case] value: &str,
    #[case] kind: UiSettingErrorKind,
    #[case] index: Option<usize>,
) {
    let input = format!(r#"{{"team_admin_editable_team_fields": {value}}}"#);
    let error = UiSettings::validate(&settings(&input)).expect_err("the value is rejected");
    assert_eq!(
        error,
        UiSettingsError {
            kind,
            field: "team_admin_editable_team_fields",
            index,
        }
    );
}

#[test]
fn record_keeps_extra_fields_for_the_patch_caller() {
    let record = UiSettings::validate(&settings(
        r#"{"enable_chat_ui": true, "enable_projects_ui": true, "nested": {"a": [1, null]}}"#,
    ))
    .expect("the settings validate");
    let dump = record.model_dump();
    assert_eq!(dump.get("enable_projects_ui"), Some(&json("true")));
    assert_eq!(dump.get("nested"), Some(&json(r#"{"a": [1, null]}"#)));
    assert_eq!(dump.len(), 18);
    assert_eq!(
        record.model_dump_exclude_unset(),
        settings(
            r#"{"enable_chat_ui": true, "enable_projects_ui": true, "nested": {"a": [1, null]}}"#
        )
    );
}

#[test]
fn record_exclude_unset_keeps_a_field_the_caller_sent_at_its_default() {
    let record = UiSettings::validate(&settings(r#"{"enable_chat_ui": false}"#))
        .expect("the settings validate");
    assert_eq!(
        record.model_dump_exclude_unset(),
        settings(r#"{"enable_chat_ui": false}"#)
    );
}

#[test]
fn schema_reports_the_pydantic_property_entries() {
    let schema = serde_json::to_value(ui_settings_schema()).expect("the schema serializes");
    assert_eq!(schema.as_object().map(|entries| entries.len()), Some(16));
    assert_eq!(
        schema["enable_chat_ui"],
        json(
            r#"{"description": "If true, shows the Chat page in the UI sidebar, letting users chat with an LLM and connect their own MCP server credentials via OAuth.", "type": "boolean"}"#
        )
    );
    assert_eq!(
        schema["enabled_ui_pages_internal_users"]["items"],
        json(r#"{"type": "string"}"#)
    );
    assert_eq!(
        schema["team_admin_editable_team_fields"]["items"],
        json(
            r#"{"enum": ["max_budget", "member_key_budgets", "projects", "rpm_limit", "tpm_limit"], "type": "string"}"#
        )
    );
}

#[test]
fn schema_team_enum_is_b1200_sorted_permission_set() {
    let mut expected = Vec::from(SUPPORTED_TEAM_FIELDS);
    expected.push(PROJECTS_PERMISSION);
    expected.push(MEMBER_KEY_BUDGETS_PERMISSION);
    expected.sort_unstable();
    let schema = serde_json::to_value(ui_settings_schema()).expect("the schema serializes");
    assert_eq!(
        schema["team_admin_editable_team_fields"]["items"]["enum"],
        json(&serde_json::to_string(&expected).expect("the names serialize"))
    );
}
