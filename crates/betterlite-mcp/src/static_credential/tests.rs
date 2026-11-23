#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::validate_static_credential;
use crate::credential_contracts::errors::CredError;
use crate::credentials::McpAuthType;

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn validate(auth_type: McpAuthType, pairs: &[(&str, &str)]) -> Result<(), CredError> {
    validate_static_credential(auth_type, &headers(pairs), None, &[])
}

fn accepts(auth_type: McpAuthType, pairs: &[(&str, &str)]) {
    assert_eq!(validate(auth_type, pairs), Ok(()));
}

fn refuses(auth_type: McpAuthType, pairs: &[(&str, &str)]) {
    validate(auth_type, pairs).unwrap_err();
}

#[rstest]
#[case::none(McpAuthType::None)]
#[case::oauth2(McpAuthType::Oauth2)]
#[case::oauth2_token_exchange(McpAuthType::Oauth2TokenExchange)]
fn non_static_modes_pass_whatever_the_headers_hold(#[case] auth_type: McpAuthType) {
    let empty = BTreeMap::new();
    assert_eq!(
        validate_static_credential(auth_type, &empty, None, &[]),
        Ok(())
    );
    accepts(auth_type, &[("Authorization", "Bearer")]);
    accepts(auth_type, &[("X-API-Key", "")]);
}

#[rstest]
#[case::opaque_key("opaque-key")]
#[case::a_bare_scheme_word("Bearer")]
#[case::a_scheme_word_followed_by_itself("Bearer Bearer")]
#[case::another_scheme_word("apikey")]
fn api_key_accepts_any_value_in_its_own_header(#[case] value: &str) {
    accepts(McpAuthType::ApiKey, &[("X-API-Key", value)]);
}

#[rstest]
#[case::lower_case("x-api-key")]
#[case::mixed_case("X-Api-Key")]
fn api_key_matches_its_header_in_any_case(#[case] name: &str) {
    accepts(McpAuthType::ApiKey, &[(name, "key")]);
}

#[rstest]
#[case::bearer_with_a_key("Bearer fixture-key")]
#[case::api_key_scheme("ApiKey fixture-key")]
#[case::token_scheme("token fixture-key")]
#[case::bearer_whose_credential_is_token("Bearer token")]
#[case::opaque("opaque-key")]
#[case::custom_word_twice("Custom Custom")]
#[case::basic_scheme("Basic dXNlcjpwYXNz")]
fn api_key_accepts_a_usable_authorization_value(#[case] value: &str) {
    accepts(McpAuthType::ApiKey, &[("Authorization", value)]);
}

#[rstest]
#[case::bearer("Bearer")]
#[case::basic("basic")]
#[case::token("token")]
#[case::apikey("apikey")]
#[case::bearer_twice("Bearer Bearer")]
#[case::bearer_repeated_in_any_case("bearer BEARER")]
#[case::token_twice("Token token")]
#[case::apikey_twice("apikey APIKEY")]
#[case::empty("")]
#[case::whitespace("   ")]
fn api_key_authorization_refuses_a_bare_or_repeated_scheme_word(#[case] value: &str) {
    refuses(McpAuthType::ApiKey, &[("Authorization", value)]);
}

#[rstest]
#[case::unrelated_header("X-Custom", "key")]
#[case::empty_default_header("X-API-Key", "")]
fn api_key_ignores_headers_outside_the_slots(#[case] name: &str, #[case] value: &str) {
    refuses(McpAuthType::ApiKey, &[(name, value)]);
}

#[test]
fn api_key_counts_an_admin_static_header() {
    let request = headers(&[("apikey", "static-key")]);
    let names = ["apikey".to_owned()];
    assert_eq!(
        validate_static_credential(McpAuthType::ApiKey, &request, None, &names),
        Ok(())
    );
}

#[test]
fn api_key_ignores_a_static_header_it_was_not_given() {
    let request = headers(&[("apikey", "static-key")]);
    let names = ["X-Tenant".to_owned()];
    validate_static_credential(McpAuthType::ApiKey, &request, None, &names).unwrap_err();
}

#[rstest]
#[case::bearer_token(McpAuthType::BearerToken)]
#[case::token(McpAuthType::Token)]
#[case::basic(McpAuthType::Basic)]
#[case::authorization(McpAuthType::Authorization)]
fn static_header_names_do_not_count_for_other_modes(#[case] auth_type: McpAuthType) {
    let request = headers(&[("apikey", "static-key")]);
    let names = ["apikey".to_owned()];
    validate_static_credential(auth_type, &request, None, &names).unwrap_err();
}

#[test]
fn a_named_upstream_header_is_a_slot() {
    let request = headers(&[("X-Tenant", "key")]);
    assert_eq!(
        validate_static_credential(McpAuthType::ApiKey, &request, Some("X-Tenant"), &[]),
        Ok(())
    );
}

#[test]
fn the_default_header_stays_a_slot_beside_a_named_one() {
    let request = headers(&[("x-api-key", "key")]);
    assert_eq!(
        validate_static_credential(McpAuthType::ApiKey, &request, Some("X-Tenant"), &[]),
        Ok(())
    );
}

#[test]
fn authorization_is_a_slot_in_every_static_mode() {
    let request = headers(&[("AUTHORIZATION", "opaque")]);
    assert_eq!(
        validate_static_credential(McpAuthType::Authorization, &request, Some("X-Tenant"), &[]),
        Ok(())
    );
}

#[test]
fn a_named_upstream_header_is_checked_by_the_same_rules() {
    let request = headers(&[("X-Custom", "Bearer")]);
    validate_static_credential(McpAuthType::Authorization, &request, Some("X-Custom"), &[])
        .unwrap_err();
}

#[test]
fn bearer_token_reads_the_named_upstream_header() {
    let request = headers(&[("X-Custom", "abc")]);
    assert_eq!(
        validate_static_credential(McpAuthType::BearerToken, &request, Some("X-Custom"), &[]),
        Ok(())
    );
}

#[test]
fn an_empty_upstream_header_name_is_a_slot_of_its_own() {
    let request = headers(&[("", "key")]);
    assert_eq!(
        validate_static_credential(McpAuthType::ApiKey, &request, Some(""), &[]),
        Ok(())
    );
}

#[rstest]
#[case::surrounding_whitespace("  Bearer fixture-key  ")]
#[case::leading_whitespace(" Bearer fixture-key")]
fn values_are_trimmed_before_the_check(#[case] value: &str) {
    accepts(McpAuthType::ApiKey, &[("Authorization", value)]);
}

#[test]
fn a_whitespace_only_value_is_empty() {
    refuses(McpAuthType::ApiKey, &[("X-API-Key", "   ")]);
    refuses(McpAuthType::Authorization, &[("Authorization", " \t ")]);
}

#[test]
fn two_headers_of_one_slot_are_two_values() {
    accepts(
        McpAuthType::ApiKey,
        &[("X-API-Key", ""), ("x-api-key", "key")],
    );
    refuses(
        McpAuthType::ApiKey,
        &[("X-API-Key", ""), ("x-api-key", "  ")],
    );
}

#[rstest]
#[case::bearer_scheme("Bearer abc")]
#[case::lower_case_scheme("bearer abc")]
#[case::no_scheme("abc")]
#[case::token_scheme("token abc")]
#[case::bearer_with_token_credential("Bearer token")]
fn bearer_token_accepts_a_credential_after_its_scheme(#[case] value: &str) {
    accepts(McpAuthType::BearerToken, &[("Authorization", value)]);
}

#[rstest]
#[case::bare_scheme("Bearer")]
#[case::scheme_twice("Bearer Bearer")]
#[case::scheme_twice_any_case("bearer BEARER")]
#[case::empty("")]
fn bearer_token_refuses_a_missing_credential(#[case] value: &str) {
    refuses(McpAuthType::BearerToken, &[("Authorization", value)]);
}

#[rstest]
#[case::token_scheme("token abc")]
#[case::bearer_scheme("Bearer abc")]
#[case::no_scheme("abc")]
fn token_accepts_a_credential_after_its_scheme(#[case] value: &str) {
    accepts(McpAuthType::Token, &[("Authorization", value)]);
}

#[rstest]
#[case::bare_scheme("token")]
#[case::scheme_twice("token token")]
#[case::scheme_twice_any_case("Token TOKEN")]
#[case::empty("")]
fn token_refuses_a_missing_credential(#[case] value: &str) {
    refuses(McpAuthType::Token, &[("Authorization", value)]);
}

#[rstest]
#[case::opaque("opaque-secret-value")]
#[case::with_a_bearer_scheme("Bearer xyz")]
#[case::with_a_basic_scheme("Basic dXNlcjpwYXNz")]
#[case::custom_words("Custom Custom")]
fn authorization_mode_accepts_any_other_value(#[case] value: &str) {
    accepts(McpAuthType::Authorization, &[("Authorization", value)]);
}

#[rstest]
#[case::bare_bearer("Bearer")]
#[case::bare_basic("Basic")]
#[case::bare_token("token")]
#[case::bare_apikey("apikey")]
#[case::empty("")]
fn authorization_mode_refuses_a_bare_scheme_word(#[case] value: &str) {
    refuses(McpAuthType::Authorization, &[("Authorization", value)]);
}

#[test]
fn authorization_mode_ignores_the_api_key_header() {
    refuses(McpAuthType::Authorization, &[("X-API-Key", "key")]);
}

#[rstest]
#[case::canonical("Basic dXNlcjpwYXNz")]
#[case::lower_scheme("basic dXNlcjpwYXNz")]
#[case::upper_scheme("BASIC dXNlcjpwYXNz")]
#[case::two_spaces("Basic  dXNlcjpwYXNz")]
#[case::non_canonical_trailing_bits("Basic dXNlcjpwYXN=")]
#[case::colon_first("Basic OnVzZXI6cGFzcw==")]
fn basic_accepts_the_scheme_and_base64_of_a_colon_pair(#[case] value: &str) {
    accepts(McpAuthType::Basic, &[("Authorization", value)]);
}

#[rstest]
#[case::no_scheme("dXNlcjpwYXNz")]
#[case::no_colon("Basic dXNlcg==")]
#[case::not_base64("Basic !!!!")]
#[case::missing_padding("Basic dXNlcjpwYXN")]
#[case::trailing_junk("Basic dXNlcjpwYXNz extra")]
#[case::inner_space("Basic dXNl cjpwYXNz")]
#[case::scheme_only("Basic")]
#[case::empty("")]
fn basic_refuses_anything_else(#[case] value: &str) {
    refuses(McpAuthType::Basic, &[("Authorization", value)]);
}

#[rstest]
#[case::api_key(McpAuthType::ApiKey, "api_key requires a usable upstream credential")]
#[case::bearer_token(
    McpAuthType::BearerToken,
    "bearer_token requires a usable upstream credential"
)]
#[case::basic(McpAuthType::Basic, "basic requires a usable upstream credential")]
#[case::token(McpAuthType::Token, "token requires a usable upstream credential")]
#[case::authorization(
    McpAuthType::Authorization,
    "authorization requires a usable upstream credential"
)]
fn a_refusal_names_the_mode(#[case] auth_type: McpAuthType, #[case] message: &str) {
    let empty = BTreeMap::new();
    let error = validate_static_credential(auth_type, &empty, None, &[]).unwrap_err();
    assert_eq!(error, CredError::Misconfigured(message.to_owned()));
}
