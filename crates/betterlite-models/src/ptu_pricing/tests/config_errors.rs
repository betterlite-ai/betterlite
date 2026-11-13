use rstest::rstest;

use super::model_info;
use crate::ptu_pricing::{PtuModelInfo, ptu_config_error};

fn config_error(source: &str) -> Option<String> {
    let info = model_info(source);
    ptu_config_error(&PtuModelInfo::from(&info), None)
}

const WINDOW: &str = r#""ptu_effective_from":"2026-01-01T00:00:00Z""#;

#[test]
fn config_error_checks_the_window_before_the_pair() {
    assert_eq!(
        config_error(
            r#"{"ptu_effective_from":"2026-06-01T00:00:00Z","ptu_effective_to":"2026-01-01T00:00:00Z"}"#
        ),
        Some("ptu_effective_to must be after ptu_effective_from".to_owned())
    );
}

#[test]
fn config_error_prefixes_the_model_name() {
    let info = model_info(
        r#"{"ptu_effective_from":"2026-06-01T00:00:00Z","ptu_effective_to":"2026-01-01T00:00:00Z"}"#,
    );
    assert_eq!(
        ptu_config_error(&PtuModelInfo::from(&info), Some("gpt-4o")),
        Some(
            "PTU configuration on model 'gpt-4o' is invalid: ptu_effective_to must be after \
             ptu_effective_from"
                .to_owned()
        )
    );
}

#[test]
fn config_error_requires_the_count_and_the_rate_together() {
    assert_eq!(
        config_error(
            r#"{"ptu_count":2,"ptu_effective_from":"2026-01-01T00:00:00Z","team_id":"team-1"}"#
        ),
        Some("ptu_count and cost_per_ptu_per_hour must be set together".to_owned())
    );
    assert_eq!(
        config_error(
            r#"{"cost_per_ptu_per_hour":1,"ptu_effective_from":"2026-01-01T00:00:00Z","team_id":"team-1"}"#
        ),
        Some("ptu_count and cost_per_ptu_per_hour must be set together".to_owned())
    );
}

#[rstest]
#[case::absent(r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1"}"#)]
#[case::invalid(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":"soon"}"#
)]
#[case::null(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":null}"#
)]
fn config_error_requires_a_parseable_start(#[case] source: &str) {
    assert_eq!(
        config_error(source),
        Some(
            "ptu_effective_from is required when PTU fields are set. Flat cost accrues from that \
             instant, so without it the start would have to be inferred and a deployment configured \
             today could be billed for days it did not exist"
                .to_owned()
        )
    );
}

#[rstest]
#[case::absent(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"ptu_effective_from":"2026-01-01T00:00:00Z"}"#
)]
#[case::empty(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"","ptu_effective_from":"2026-01-01T00:00:00Z"}"#
)]
#[case::null(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":null,"ptu_effective_from":"2026-01-01T00:00:00Z"}"#
)]
#[case::wrong_type(
    r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":7,"ptu_effective_from":"2026-01-01T00:00:00Z"}"#
)]
fn config_error_requires_a_team(#[case] source: &str) {
    assert_eq!(
        config_error(source),
        Some("team_id is required when PTU fields are set (one model maps to one team)".to_owned())
    );
}

#[test]
fn config_error_accepts_a_complete_declaration() {
    let source =
        format!(r#"{{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1",{WINDOW}}}"#);
    assert_eq!(config_error(&source), None);
}

// The update-shape check does not repeat ptu_terms' numeric bounds, so these still read as configured.
#[rstest]
#[case::zero_count(r#""ptu_count":0,"cost_per_ptu_per_hour":1"#)]
#[case::zero_rate(r#""ptu_count":2,"cost_per_ptu_per_hour":0"#)]
#[case::negative_rate(r#""ptu_count":2,"cost_per_ptu_per_hour":-5"#)]
#[case::unusable_count(r#""ptu_count":"many","cost_per_ptu_per_hour":1"#)]
fn config_error_ignores_chargeability(#[case] pair: &str) {
    let source = format!(r#"{{{pair},"team_id":"team-1",{WINDOW}}}"#);
    assert_eq!(config_error(&source), None);
}

#[test]
fn config_error_ignores_a_null_end() {
    let source = format!(
        r#"{{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1",{WINDOW},"ptu_effective_to":null}}"#
    );
    assert_eq!(config_error(&source), None);
}

#[test]
fn config_error_does_not_check_the_window_when_the_start_fails_to_parse() {
    assert_eq!(
        config_error(
            r#"{"ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":"soon","ptu_effective_to":"2025-01-01T00:00:00Z"}"#
        ),
        Some(
            "ptu_effective_from is required when PTU fields are set. Flat cost accrues from that \
             instant, so without it the start would have to be inferred and a deployment configured \
             today could be billed for days it did not exist"
                .to_owned()
        )
    );
}

#[test]
fn config_error_returns_none_when_no_ptu_field_is_declared() {
    assert_eq!(config_error("{}"), None);
    assert_eq!(config_error(r#"{"team_id":"team-1"}"#), None);
    assert_eq!(
        config_error(r#"{"ptu_count":null,"cost_per_ptu_per_hour":null}"#),
        None
    );
}
