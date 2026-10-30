#![cfg(test)]

use super::Input;
use rstest::rstest;

#[rstest]
#[case::valid(
    r#"{"operation":"canonical_lookup","source":{},"lookup":["model"]}"#,
    true
)]
#[case::unknown_field(r#"{"operation":"packaged_map","extra":true}"#, false)]
#[case::unknown_operation(r#"{"operation":"not_supported"}"#, false)]
#[case::non_string_lookup(r#"{"operation":"canonical_lookup","lookup":[42]}"#, false)]
fn typed_catalog_input(#[case] text: &str, #[case] valid: bool) {
    assert_eq!(serde_json::from_str::<Input>(text).is_ok(), valid);
}
