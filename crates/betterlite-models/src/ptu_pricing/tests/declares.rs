use rstest::rstest;

use super::model_info;
use crate::ptu_pricing::{PtuModelInfo, declares_ptu};

fn declares(source: &str) -> bool {
    let info = model_info(source);
    declares_ptu(&PtuModelInfo::from(&info))
}

#[test]
fn declares_ptu_is_false_without_a_ptu_field() {
    assert!(!declares("{}"));
    assert!(!declares(r#"{"team_id":"team-1"}"#));
}

#[rstest]
#[case::count(r#"{"ptu_count":2}"#)]
#[case::rate(r#"{"cost_per_ptu_per_hour":1}"#)]
#[case::start(r#"{"ptu_effective_from":"2026-01-01"}"#)]
#[case::end(r#"{"ptu_effective_to":"2026-01-01"}"#)]
fn declares_ptu_is_true_for_a_set_field(#[case] source: &str) {
    assert!(declares(source));
}

#[test]
fn declares_ptu_ignores_null_fields() {
    assert!(!declares(
        r#"{"ptu_count":null,"cost_per_ptu_per_hour":null,"ptu_effective_from":null,"ptu_effective_to":null}"#
    ));
}

#[test]
fn declares_ptu_reads_a_malformed_value_as_declared() {
    assert!(declares(r#"{"ptu_count":"many"}"#));
    assert!(declares(r#"{"ptu_effective_from":7}"#));
}
