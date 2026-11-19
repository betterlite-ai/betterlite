#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{
    CostTrackingService, END_USER_ID_FIELD, EndUserCostTrackingSettings,
    get_end_user_id_for_cost_tracking,
};

fn params(encoded: &str) -> JsonObject {
    serde_json::from_str(encoded).expect("litellm params")
}

fn settings(disable: Option<bool>, prometheus_only: Option<bool>) -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: disable,
        enable_prometheus_only: prometheus_only,
    }
}

#[rstest]
#[case::top_level_wins(
    r#"{"user_api_key_end_user_id": "top", "metadata": {"user_api_key_end_user_id": "merged"}}"#,
    Some("top")
)]
#[case::empty_top_level_falls_back(
    r#"{"user_api_key_end_user_id": "", "metadata": {"user_api_key_end_user_id": "merged"}}"#,
    Some("merged")
)]
#[case::merged_when_absent(
    r#"{"metadata": {"user_api_key_end_user_id": "merged"}}"#,
    Some("merged")
)]
#[case::litellm_metadata_bucket(
    r#"{"litellm_metadata": {"user_api_key_end_user_id": "internal"}}"#,
    Some("internal")
)]
#[case::nothing_recorded("{}", None)]
#[case::null_top_level_falls_back(
    r#"{"user_api_key_end_user_id": null, "metadata": {"user_api_key_end_user_id": "merged"}}"#,
    Some("merged")
)]
#[case::empty_everywhere_stays_empty(r#"{"metadata": {"user_api_key_end_user_id": ""}}"#, Some(""))]
fn resolves_the_end_user_id(#[case] encoded: &str, #[case] expected: Option<&str>) {
    let resolved = get_end_user_id_for_cost_tracking(
        &params(encoded),
        CostTrackingService::LitellmLogging,
        &settings(None, None),
    )
    .expect("end user id");
    assert_eq!(resolved.as_deref(), expected);
}

#[rstest]
#[case::disabled_suppresses(Some(true), None)]
#[case::enabled_keeps(Some(false), Some("user"))]
#[case::unset_keeps(None, Some("user"))]
fn the_disable_setting_gates_every_service(
    #[case] disable: Option<bool>,
    #[case] expected: Option<&str>,
) {
    for service in [
        CostTrackingService::LitellmLogging,
        CostTrackingService::Prometheus,
    ] {
        let enabled_only = (service == CostTrackingService::Prometheus).then_some(true);
        let resolved = get_end_user_id_for_cost_tracking(
            &params(r#"{"user_api_key_end_user_id": "user"}"#),
            service,
            &settings(disable, enabled_only),
        )
        .expect("end user id");
        assert_eq!(resolved.as_deref(), expected);
    }
}

#[rstest]
#[case::unset(None, None)]
#[case::flagged_off(Some(false), None)]
#[case::flagged_on(Some(true), Some("user"))]
fn prometheus_needs_the_exact_enable_flag(
    #[case] prometheus_only: Option<bool>,
    #[case] expected: Option<&str>,
) {
    let resolved = get_end_user_id_for_cost_tracking(
        &params(r#"{"user_api_key_end_user_id": "user"}"#),
        CostTrackingService::Prometheus,
        &settings(None, prometheus_only),
    )
    .expect("end user id");
    assert_eq!(resolved.as_deref(), expected);
}

#[test]
fn the_prometheus_flag_does_not_gate_spend_logging() {
    let resolved = get_end_user_id_for_cost_tracking(
        &params(r#"{"user_api_key_end_user_id": "user"}"#),
        CostTrackingService::LitellmLogging,
        &settings(Some(false), Some(false)),
    )
    .expect("end user id");
    assert_eq!(resolved.as_deref(), Some("user"));
}

#[test]
fn the_id_is_read_before_the_disable_check() {
    let error = get_end_user_id_for_cost_tracking(
        &params(r#"{"user_api_key_end_user_id": 7}"#),
        CostTrackingService::LitellmLogging,
        &settings(Some(true), None),
    )
    .expect_err("a non-string id is out of type");
    assert!(error.to_string().contains(END_USER_ID_FIELD));
}

#[rstest]
#[case::top_level(r#"{"user_api_key_end_user_id": ["user"]}"#)]
#[case::merged(r#"{"metadata": {"user_api_key_end_user_id": 5}}"#)]
fn a_non_string_id_names_the_field(#[case] encoded: &str) {
    let error = get_end_user_id_for_cost_tracking(
        &params(encoded),
        CostTrackingService::LitellmLogging,
        &settings(None, None),
    )
    .expect_err("a non-string id is out of type");
    assert!(error.to_string().contains(END_USER_ID_FIELD));
}
