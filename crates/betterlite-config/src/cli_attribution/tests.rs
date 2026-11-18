#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::cli_claim_map;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::primary_only(&[("CLI_SSO_CLAIM_MAP", "a->b")], "a->b")]
#[case::fallback_only(&[("LITELLM_CLI_SSO_CLAIM_MAP", "a->b")], "a->b")]
#[case::primary_wins(
    &[("CLI_SSO_CLAIM_MAP", "x->y"), ("LITELLM_CLI_SSO_CLAIM_MAP", "a->b")],
    "x->y"
)]
#[case::empty_primary_uses_fallback(
    &[("CLI_SSO_CLAIM_MAP", ""), ("LITELLM_CLI_SSO_CLAIM_MAP", "a->b")],
    "a->b"
)]
#[case::empty_fallback_keeps_primary(
    &[("CLI_SSO_CLAIM_MAP", "x->y"), ("LITELLM_CLI_SSO_CLAIM_MAP", "")],
    "x->y"
)]
#[case::both_empty(&[("CLI_SSO_CLAIM_MAP", ""), ("LITELLM_CLI_SSO_CLAIM_MAP", "")], "")]
#[case::unset(&[], "")]
#[case::whitespace_primary(
    &[("CLI_SSO_CLAIM_MAP", " "), ("LITELLM_CLI_SSO_CLAIM_MAP", "a->b")],
    " "
)]
#[case::map_text_kept_verbatim(
    &[(
        "CLI_SSO_CLAIM_MAP",
        "employment_type->acme_employment_type,org_info.department->department",
    )],
    "employment_type->acme_employment_type,org_info.department->department"
)]
fn loads_cli_claim_map(#[case] pairs: &[(&str, &str)], #[case] expected: &str) {
    assert_eq!(cli_claim_map(&environment(pairs)), expected);
}
