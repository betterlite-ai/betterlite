use betterlite_config::sections::settings::{JsonValue, Section};
use rstest::rstest;

use super::support::json;
use crate::config_boot::test_support::capture_logs;
use crate::db_config_overlay::scrub::scrub_remote_modules;

#[rstest]
#[case::general_custom_auth(Section::GeneralSettings, "custom_auth", "s3://bucket/auth.py")]
#[case::general_key_generate(
    Section::GeneralSettings,
    "custom_key_generate",
    "gcs://bucket/keys.py"
)]
#[case::general_key_update(Section::GeneralSettings, "custom_key_update", "s3://bucket/update.py")]
#[case::general_key_policy(Section::GeneralSettings, "custom_key_policy", "s3://bucket/policy.py")]
#[case::general_team_metadata(
    Section::GeneralSettings,
    "custom_team_metadata_validate",
    "s3://bucket/team.py"
)]
#[case::general_sso(Section::GeneralSettings, "custom_sso", "gcs://bucket/sso.py")]
#[case::general_sign_in(
    Section::GeneralSettings,
    "custom_ui_sso_sign_in_handler",
    "s3://bucket/sign_in.py"
)]
#[case::litellm_post_call_rules(
    Section::LitellmSettings,
    "post_call_rules",
    "gcs://bucket/rules.py"
)]
fn scrub_nulls_a_remote_module_field(
    #[case] section: Section,
    #[case] field: &str,
    #[case] module: &str,
) {
    let (logs, _guard) = capture_logs();
    let mut row = json!({"keep": "local.module"});
    row[field] = JsonValue::from(module);
    let scrubbed = scrub_remote_modules(section, &row);
    assert_eq!(scrubbed[field], JsonValue::Null);
    assert_eq!(scrubbed["keep"], json!("local.module"));
    assert!(logs.contents().contains(&format!(
        "Refused remote-URL value for DB-overlay {}.{field}=\"{module}\"; only config.yaml entries may reference s3:// / gcs:// modules.",
        section.name()
    )));
}

#[rstest]
#[case::callbacks("callbacks")]
#[case::success_callback("success_callback")]
#[case::failure_callback("failure_callback")]
#[case::audit_log_callbacks("audit_log_callbacks")]
fn scrub_drops_remote_list_entries(#[case] field: &str) {
    let (logs, _guard) = capture_logs();
    let mut row = json!({});
    row[field] = json!(["s3://bucket/a.py", "langfuse", "gcs://bucket/b.py", 7]);
    let scrubbed = scrub_remote_modules(Section::LitellmSettings, &row);
    assert_eq!(scrubbed[field], json!(["langfuse", 7]));
    assert!(logs.contents().contains(&format!(
        "Refused 2 remote-URL entries from DB-overlay litellm_settings.{field}; only config.yaml entries may reference s3:// / gcs:// modules."
    )));
}

#[test]
fn scrub_leaves_a_clean_list_without_a_warning() {
    let (logs, _guard) = capture_logs();
    let row = json!({"callbacks": ["langfuse"], "custom_auth": "s3://bucket/auth.py"});
    let scrubbed = scrub_remote_modules(Section::LitellmSettings, &row);
    assert_eq!(scrubbed, row);
    assert_eq!(logs.contents(), "");
}

#[test]
fn scrub_nulls_remote_custom_handlers() {
    let (logs, _guard) = capture_logs();
    let row = json!({"custom_provider_map": [
        {"provider": "a", "custom_handler": "s3://bucket/handler.py"},
        {"provider": "b", "custom_handler": "local.handler"},
        "not-a-mapping"
    ]});
    let scrubbed = scrub_remote_modules(Section::LitellmSettings, &row);
    assert_eq!(
        scrubbed["custom_provider_map"],
        json!([
            {"provider": "a", "custom_handler": null},
            {"provider": "b", "custom_handler": "local.handler"},
            "not-a-mapping"
        ])
    );
    assert!(logs.contents().contains(
        "Refused remote-URL custom_handler from DB-overlay litellm_settings.custom_provider_map: \"s3://bucket/handler.py\""
    ));
}

#[test]
fn scrub_cleans_both_guardrail_shapes() {
    let (logs, _guard) = capture_logs();
    let row = json!({"guardrails": [
        {"v1": {"callbacks": ["s3://bucket/g.py", "presidio"], "default_on": true}},
        {"guardrail_name": "v2", "litellm_params": {"guardrail": "gcs://bucket/g.py", "callbacks": ["gcs://x"]}},
        "skipped"
    ]});
    let scrubbed = scrub_remote_modules(Section::LitellmSettings, &row);
    assert_eq!(
        scrubbed["guardrails"],
        json!([
            {"v1": {"callbacks": ["presidio"], "default_on": true}},
            {"guardrail_name": "v2", "litellm_params": {"guardrail": null, "callbacks": []}},
            "skipped"
        ])
    );
    let contents = logs.contents();
    assert_eq!(
        contents
            .matches("Refused 1 remote-URL entries from DB-overlay litellm_settings.guardrails[...].callbacks")
            .count(),
        2
    );
    assert!(contents.contains(
        "Refused remote-URL guardrail module from DB-overlay litellm_settings.guardrails[...].guardrail: \"gcs://bucket/g.py\""
    ));
}

#[test]
fn scrub_nulls_a_remote_jwt_validator() {
    let (logs, _guard) = capture_logs();
    let row = json!({"litellm_jwtauth": {"custom_validate": "s3://bucket/jwt.py", "user_id_jwt_field": "sub"}});
    let scrubbed = scrub_remote_modules(Section::GeneralSettings, &row);
    assert_eq!(
        scrubbed["litellm_jwtauth"],
        json!({"custom_validate": null, "user_id_jwt_field": "sub"})
    );
    assert!(logs.contents().contains(
        "Refused remote-URL custom_validate from DB-overlay general_settings.litellm_jwtauth: \"s3://bucket/jwt.py\""
    ));
}

#[test]
fn scrub_nulls_remote_pass_through_targets() {
    let (logs, _guard) = capture_logs();
    let row = json!({"pass_through_endpoints": [
        {"path": "/remote", "target": "s3://bucket/m.handler"},
        {"target": "gcs://bucket/m.handler"},
        {"path": "/http", "target": "https://example.test"}
    ]});
    let scrubbed = scrub_remote_modules(Section::GeneralSettings, &row);
    assert_eq!(
        scrubbed["pass_through_endpoints"],
        json!([
            {"path": "/remote", "target": null},
            {"target": null},
            {"path": "/http", "target": "https://example.test"}
        ])
    );
    let contents = logs.contents();
    assert!(contents.contains(
        "Refused remote-URL target from DB-overlay general_settings.pass_through_endpoints (path=\"/remote\"): \"s3://bucket/m.handler\""
    ));
    assert!(contents.contains(
        "Refused remote-URL target from DB-overlay general_settings.pass_through_endpoints (path=null): \"gcs://bucket/m.handler\""
    ));
}

#[rstest]
#[case::router(Section::RouterSettings)]
#[case::environment(Section::EnvironmentVariables)]
#[case::ui(Section::UiSettings)]
fn scrub_leaves_other_sections_alone(#[case] section: Section) {
    let row = json!({"custom_auth": "s3://bucket/auth.py", "callbacks": ["s3://bucket/a.py"]});
    assert_eq!(scrub_remote_modules(section, &row), row);
}

#[rstest]
#[case::uppercase_scheme("S3://bucket/auth.py")]
#[case::single_slash("s3:/bucket/auth.py")]
#[case::gs_scheme("gs://bucket/auth.py")]
#[case::https("https://example.test/auth.py")]
fn scrub_keeps_values_that_are_not_remote_modules(#[case] module: &str) {
    let mut row = json!({});
    row["custom_auth"] = JsonValue::from(module);
    assert_eq!(scrub_remote_modules(Section::GeneralSettings, &row), row);
}

#[rstest]
#[case::text(json!("s3://bucket/auth.py"))]
#[case::list(json!(["s3://bucket/auth.py"]))]
#[case::null(json!(null))]
fn scrub_returns_a_row_that_is_not_a_mapping(#[case] row: JsonValue) {
    assert_eq!(scrub_remote_modules(Section::GeneralSettings, &row), row);
}

#[test]
fn scrub_leaves_the_stored_row_unchanged() {
    let row = json!({"custom_auth": "s3://bucket/auth.py"});
    let before = row.clone();
    let scrubbed = scrub_remote_modules(Section::GeneralSettings, &row);
    assert_eq!(row, before);
    assert_ne!(scrubbed, row);
}
