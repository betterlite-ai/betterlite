#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{GOOGLE_CLIENT_SECRET_ENV, load_google_sso_settings};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(&[], None)]
#[case::present(&[(GOOGLE_CLIENT_SECRET_ENV, "g-secret")], Some("g-secret"))]
#[case::empty_is_unset(&[(GOOGLE_CLIENT_SECRET_ENV, "")], None)]
#[case::spaces_are_kept(&[(GOOGLE_CLIENT_SECRET_ENV, " ")], Some(" "))]
fn loads_google_client_secret(#[case] pairs: &[(&str, &str)], #[case] expected: Option<&str>) {
    let settings = load_google_sso_settings(&environment(pairs));
    let loaded = settings
        .client_secret
        .as_ref()
        .map(|secret| secret.expose_secret());
    assert_eq!(loaded, expected);
}
