#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    GENERIC_CLIENT_STATE_ENV, GENERIC_CLIENT_USE_PKCE_ENV, PKCE_STRICT_CACHE_MISS_ENV,
    PkceSettings, load_pkce_settings,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[rstest]
#[case::nothing_set(&[], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_on(&[(GENERIC_CLIENT_USE_PKCE_ENV, "true")], PkceSettings { use_pkce: true, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_on_in_upper_case(&[(GENERIC_CLIENT_USE_PKCE_ENV, "TRUE")], PkceSettings { use_pkce: true, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_on_in_mixed_case(&[(GENERIC_CLIENT_USE_PKCE_ENV, "True")], PkceSettings { use_pkce: true, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_off_for_one(&[(GENERIC_CLIENT_USE_PKCE_ENV, "1")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_off_for_yes(&[(GENERIC_CLIENT_USE_PKCE_ENV, "yes")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_off_for_padding(&[(GENERIC_CLIENT_USE_PKCE_ENV, " true")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::pkce_off_for_empty(&[(GENERIC_CLIENT_USE_PKCE_ENV, "")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::strict_on(&[(PKCE_STRICT_CACHE_MISS_ENV, "true")], PkceSettings { use_pkce: false, strict_cache_miss: true, generic_client_state: None })]
#[case::strict_on_in_upper_case(&[(PKCE_STRICT_CACHE_MISS_ENV, "TRUE")], PkceSettings { use_pkce: false, strict_cache_miss: true, generic_client_state: None })]
#[case::strict_off_for_on(&[(PKCE_STRICT_CACHE_MISS_ENV, "on")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::configured_state(&[(GENERIC_CLIENT_STATE_ENV, "configured")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: Some("configured".to_owned()) })]
#[case::configured_state_keeps_spaces(&[(GENERIC_CLIENT_STATE_ENV, " ") ], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: Some(" ".to_owned()) })]
#[case::empty_configured_state_is_absent(&[(GENERIC_CLIENT_STATE_ENV, "")], PkceSettings { use_pkce: false, strict_cache_miss: false, generic_client_state: None })]
#[case::all_three_set(
    &[(GENERIC_CLIENT_USE_PKCE_ENV, "true"), (PKCE_STRICT_CACHE_MISS_ENV, "true"), (GENERIC_CLIENT_STATE_ENV, "both")],
    PkceSettings { use_pkce: true, strict_cache_miss: true, generic_client_state: Some("both".to_owned()) }
)]
fn loads_pkce_settings(#[case] pairs: &[(&str, &str)], #[case] expected: PkceSettings) {
    assert_eq!(load_pkce_settings(&environment(pairs)), expected);
}
