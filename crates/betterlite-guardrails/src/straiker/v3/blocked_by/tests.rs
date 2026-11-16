#![cfg(test)]

use rstest::rstest;

use super::blocking_controls;
use crate::straiker::v3::fixtures::object;

#[rstest]
#[case::sorted(r#"{"blocked_by":["pii","jailbreak","exfil"]}"#, &["exfil", "jailbreak", "pii"])]
#[case::duplicates_are_kept(r#"{"blocked_by":["b","a","b"]}"#, &["a", "b", "b"])]
#[case::single(r#"{"blocked_by":["only"]}"#, &["only"])]
#[case::empty_list(r#"{"blocked_by":[]}"#, &[])]
#[case::not_a_list(r#"{"blocked_by":"pii"}"#, &[])]
#[case::absent(r#"{"action":"BLOCKED"}"#, &[])]
fn lists_blocking_controls(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(blocking_controls(&object(source)), expected);
}
