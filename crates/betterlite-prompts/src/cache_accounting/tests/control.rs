use rstest::rstest;

use crate::cache_accounting::control::default_control;

fn rendered(ttl: Option<&str>) -> String {
    serde_json::to_string(&default_control(ttl)).unwrap()
}

#[rstest]
#[case::unset(None, r#"{"type":"ephemeral"}"#)]
#[case::five_minutes(Some("5m"), r#"{"type":"ephemeral","ttl":"5m"}"#)]
#[case::one_hour(Some("1h"), r#"{"type":"ephemeral","ttl":"1h"}"#)]
#[case::other_value(Some("2h"), r#"{"type":"ephemeral"}"#)]
#[case::empty_value(Some(""), r#"{"type":"ephemeral"}"#)]
#[case::wrong_case(Some("1H"), r#"{"type":"ephemeral"}"#)]
#[case::longer_value(Some("1 hour"), r#"{"type":"ephemeral"}"#)]
fn default_control_keeps_only_the_two_ttl_spellings(
    #[case] ttl: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(rendered(ttl), expected);
}
