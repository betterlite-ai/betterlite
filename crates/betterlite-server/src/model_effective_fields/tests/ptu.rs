use rstest::rstest;

use crate::model_effective_fields::error::ModelFieldError;
use crate::model_effective_fields::ptu::{
    check_ptu_fields_allowed, check_ptu_model_info, explicitly_cleared_ptu_fields,
    merged_ptu_model_info,
};

use super::support::{Attribution, object};

const STORED: &str = r#"{"id": "m1", "team_id": "t1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "ptu_effective_from": "2026-01-01T00:00:00Z", "ptu_effective_to": "2026-06-01T00:00:00Z"}"#;
const CLEARING: &str = r#"{"ptu_count": null, "cost_per_ptu_per_hour": null, "team_id": null, "tier": null, "ptu_effective_to": "2026-09-01T00:00:00Z", "base_model": "gpt-4o"}"#;

#[rstest]
#[case::enabled_clears_sent_nulls(
    true,
    r#"["ptu_count", "cost_per_ptu_per_hour"]"#,
    r#"{"id": "m1", "team_id": "t1", "ptu_effective_from": "2026-01-01T00:00:00Z", "ptu_effective_to": "2026-09-01T00:00:00Z", "base_model": "gpt-4o"}"#
)]
#[case::disabled_keeps_what_was_stored(
    false,
    "[]",
    r#"{"id": "m1", "team_id": "t1", "ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "ptu_effective_from": "2026-01-01T00:00:00Z", "ptu_effective_to": "2026-09-01T00:00:00Z", "base_model": "gpt-4o"}"#
)]
fn null_clearable_fields(#[case] enabled: bool, #[case] cleared: &str, #[case] merged: &str) {
    let attribution = Attribution(enabled);
    let incoming = object(CLEARING);
    let cleared_fields = explicitly_cleared_ptu_fields(Some(&incoming), &attribution);
    let expected: Vec<String> = serde_json::from_str(cleared).unwrap();
    assert_eq!(cleared_fields, expected);
    let result = merged_ptu_model_info(&object(STORED), Some(&incoming), &attribution);
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        serde_json::to_string(&object(merged)).unwrap()
    );
}

#[test]
fn nothing_is_cleared_without_a_model_info() {
    let attribution = Attribution(true);
    assert!(explicitly_cleared_ptu_fields(None, &attribution).is_empty());
    let stored = object(r#"{"id": "m1", "tier": null}"#);
    let merged = merged_ptu_model_info(&stored, None, &attribution);
    assert_eq!(merged, object(r#"{"id": "m1"}"#));
}

#[rstest]
#[case::enabled_accepts_values(true, r#"{"ptu_count": 2}"#, None)]
#[case::disabled_accepts_nulls(false, r#"{"ptu_count": null, "team_id": "t1"}"#, None)]
#[case::disabled_lists_every_value(
    false,
    r#"{"ptu_effective_to": "2026-02-01T00:00:00Z", "ptu_count": 2, "cost_per_ptu_per_hour": 0}"#,
    Some("ptu_count, cost_per_ptu_per_hour, ptu_effective_to")
)]
#[case::disabled_refuses_a_start(
    false,
    r#"{"ptu_effective_from": "2026-01-01T00:00:00Z"}"#,
    Some("ptu_effective_from")
)]
fn ptu_fields_need_attribution(
    #[case] enabled: bool,
    #[case] incoming: &str,
    #[case] refused: Option<&str>,
) {
    let outcome = check_ptu_fields_allowed(&object(incoming), &Attribution(enabled));
    let Some(fields) = refused else {
        outcome.unwrap();
        return;
    };
    let error = outcome.unwrap_err();
    assert!(matches!(error, ModelFieldError::PtuDisabled { .. }));
    assert_eq!(error.refusal().status, 400);
    assert_eq!(error.refusal().error_type, None);
    assert_eq!(error.refusal().param, None);
    assert_eq!(
        error.to_string(),
        format!(
            "PTU cost attribution is disabled, so {fields} cannot be set. Set LITELLM_ENABLE_PTU_COST_ATTRIBUTION=true to enable it."
        )
    );
}

#[rstest]
#[case::no_ptu(r#"{"id": "m1"}"#, None)]
#[case::complete(STORED, None)]
#[case::count_without_rate(
    r#"{"team_id": "t1", "ptu_count": 2, "ptu_effective_from": "2026-01-01T00:00:00Z"}"#,
    Some("ptu_count and cost_per_ptu_per_hour must be set together")
)]
#[case::inverted_window(
    r#"{"ptu_effective_from": "2026-06-01T00:00:00Z", "ptu_effective_to": "2026-01-01T00:00:00Z"}"#,
    Some("ptu_effective_to must be after ptu_effective_from")
)]
#[case::missing_team(
    r#"{"ptu_count": 2, "cost_per_ptu_per_hour": 1.5, "ptu_effective_from": "2026-01-01T00:00:00Z", "team_id": ["t1"]}"#,
    Some("team_id is required when PTU fields are set (one model maps to one team)")
)]
fn ptu_model_info_is_judged_whole(#[case] model_info: &str, #[case] refused: Option<&str>) {
    let outcome = check_ptu_model_info(&object(model_info));
    let Some(message) = refused else {
        outcome.unwrap();
        return;
    };
    let error = outcome.unwrap_err();
    assert!(matches!(error, ModelFieldError::PtuInvalid { .. }));
    assert_eq!(error.refusal().status, 400);
    assert_eq!(error.to_string(), message);
}
