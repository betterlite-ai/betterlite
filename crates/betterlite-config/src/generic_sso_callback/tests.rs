#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{GenericSsoCallbackSettings, INCLUDE_TOKEN_CLAIMS_ENV, load_callback_settings};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[rstest]
#[case::nothing_set(&[], GenericSsoCallbackSettings { include_token_claims: false })]
#[case::enabled(&[(INCLUDE_TOKEN_CLAIMS_ENV, "true")], GenericSsoCallbackSettings { include_token_claims: true })]
#[case::enabled_in_upper_case(&[(INCLUDE_TOKEN_CLAIMS_ENV, "TRUE")], GenericSsoCallbackSettings { include_token_claims: true })]
#[case::enabled_in_mixed_case(&[(INCLUDE_TOKEN_CLAIMS_ENV, "True")], GenericSsoCallbackSettings { include_token_claims: true })]
#[case::off_for_one(&[(INCLUDE_TOKEN_CLAIMS_ENV, "1")], GenericSsoCallbackSettings { include_token_claims: false })]
#[case::off_for_yes(&[(INCLUDE_TOKEN_CLAIMS_ENV, "yes")], GenericSsoCallbackSettings { include_token_claims: false })]
#[case::off_for_padding(&[(INCLUDE_TOKEN_CLAIMS_ENV, " true")], GenericSsoCallbackSettings { include_token_claims: false })]
#[case::off_for_explicit_false(&[(INCLUDE_TOKEN_CLAIMS_ENV, "false")], GenericSsoCallbackSettings { include_token_claims: false })]
#[case::off_for_empty(&[(INCLUDE_TOKEN_CLAIMS_ENV, "")], GenericSsoCallbackSettings { include_token_claims: false })]
fn loads_include_token_claims(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: GenericSsoCallbackSettings,
) {
    assert_eq!(load_callback_settings(&environment(pairs)), expected);
}
