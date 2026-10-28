#![cfg(test)]

use super::SensitiveDataMasker;
use rstest::rstest;

#[rstest]
#[case::hyphen("API-KEY", &[], true)]
#[case::unicode("\u{212a}ey", &[], true)]
#[case::plural("vertex_credentials", &[], true)]
#[case::tenancy("oci_tenancy", &[], true)]
#[case::ordinary("model", &[], false)]
#[case::substring("max_tokens", &[], false)]
#[case::cost_precedence("input_cost_per_token", &[], false)]
#[case::custom("database_connection", &[], true)]
#[case::excluded("api_key", &["api_key"], false)]
#[case::exclusion_case("API_KEY", &["api_key"], true)]
fn structured_redaction_behavior_tests_behavior(
    #[case] key: &str,
    #[case] excluded: &[&str],
    #[case] expected: bool,
) {
    let masker = SensitiveDataMasker::with_patterns(["connection"], ["cost"]);
    let exclusions = excluded
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(masker.is_sensitive_key(key, &exclusions), expected);
}
