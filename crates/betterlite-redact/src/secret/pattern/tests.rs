#![cfg(test)]

use std::time::{Duration, Instant};

use rstest::rstest;

use crate::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

#[test]
fn replacement_span_prefers_the_named_guarded_group_over_the_whole_match() {
    let patterns = super::build(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = "?key=secretvalue";
    let captures = patterns.whole.captures_at(text, 0).unwrap();
    let whole = captures.get(0).unwrap();
    assert_eq!(
        patterns.replacement_span(text, &captures, whole),
        Some((1, text.len()))
    );
}

#[test]
fn replacement_span_falls_back_to_the_whole_match_when_nothing_is_guarded() {
    let patterns = super::build(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = "Bearer abcdefghij1234567890";
    let captures = patterns.whole.captures_at(text, 0).unwrap();
    let whole = captures.get(0).unwrap();
    assert_eq!(
        patterns.replacement_span(text, &captures, whole),
        Some((0, text.len()))
    );
}

#[rstest]
#[case::pem_private_key_block(
    "cert=-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----",
    "cert=REDACTED"
)]
#[case::gcp_ya29_access_token("token ya29.a0AfH6SMB_long_token_value", "token REDACTED")]
#[case::client_secret_space_separated("client_secret abcDEF123456", "REDACTED")]
#[case::aws_akia_access_key("AKIA0123456789ABCDEF", "REDACTED")]
#[case::bearer_token(
    "Authorization: Bearer abcdefghij1234567890==",
    "Authorization: REDACTED"
)]
#[case::basic_auth_header(
    "Authorization: Basic abcdefghij1234567890==",
    "Authorization: REDACTED"
)]
#[case::openai_sk_prefixed_key("sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "REDACTED")]
#[case::url_query_token_keeps_leading_delimiter(
    "GET /v1?token=AAAAAAAAAA HTTP/1.1",
    "GET /v1?REDACTED HTTP/1.1"
)]
#[case::url_query_multiple_params_both_redacted(
    "?token=AAAAAAAAAA&password=BBBBBBBBBB",
    "?REDACTED&REDACTED"
)]
#[case::generic_api_key_kv("api_key='sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'", "REDACTED'")]
#[case::x_api_key_header("x-api-key: abcdefghij", "REDACTED")]
#[case::anthropic_x_ak_header("x-ak-aaaaaaaaaaaaaaaaaaaaaa", "REDACTED")]
#[case::url_key_param_keeps_leading_delimiter("&key=AAAAAAAAAAAAAAAA", "&REDACTED")]
#[case::generic_password_kv("password='hunter2hunter2'", "REDACTED'")]
#[case::database_connection_string(
    "postgres://dbuser:dbpass123@db.internal:5432/app",
    "postgres://REDACTED@db.internal:5432/app"
)]
// Split so GitHub push protection does not take the fixture for a real Databricks token.
#[case::databricks_dapi_token(concat!("dapi", "0123456789abcdef0123456789abcdef"), "REDACTED")]
#[case::litellm_provider_key("litellm.openai_key='sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'", "REDACTED'")]
#[case::private_key_pem_value(
    "private_key=-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----",
    "REDACTED"
)]
#[case::master_key_name("master_key='sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'", "REDACTED'")]
#[case::master_key_name_shadowed_by_generic_token_falls_back(
    "aws_session_token=zzzzyyyyxxxxwwww",
    "REDACTED"
)]
#[case::raw_jwt("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj", "REDACTED")]
#[case::azure_sas_signature_keeps_leading_delimiter(
    "?sig=abc%2Fdef%3D&se=2024",
    "?REDACTED&se=2024"
)]
#[case::azure_sig_lookalike_without_delimiter_not_redacted(
    "the value sig=abcdefgh appears here",
    "the value sig=abcdefgh appears here"
)]
#[case::service_account_json_blob(
    r#"creds={"type": "service_account", "project_id": "x"}"#,
    "creds=REDACTED"
)]
#[case::mixed_case_akia_still_matches("akia0123456789abcdef", "REDACTED")]
#[case::mixed_case_bearer_still_matches("BEARER abcdefghij1234567890", "REDACTED")]
#[case::benign_lookalike_mytoken_not_redacted(
    "mytoken=abc123 status=ok",
    "mytoken=abc123 status=ok"
)]
#[case::benign_lookalike_plain_word_token_not_bare_secret(
    "the token system worked",
    "the token system worked"
)]
#[case::url_query_cred_rejection_falls_through_to_db_conn(
    "postgres://user_token=x:pass@host/db",
    "postgres://REDACTED@host/db"
)]
#[case::url_key_rejection_falls_through_to_db_conn(
    "redis://key=abcdefgh:hunter2@host",
    "redis://REDACTED@host"
)]
#[case::db_conn_userinfo_starting_with_query_delimiter_is_not_guard_elevated(
    "postgres://?key=abcdefghij:pw@host/db",
    "postgres://REDACTED@host/db"
)]
#[case::db_conn_userinfo_starting_with_amp_delimiter_is_not_guard_elevated(
    "redis://&token=abc:pw@host/db",
    "redis://REDACTED@host/db"
)]
#[case::db_conn_credential_boundary_redacts_up_to_the_last_at_sign(
    "postgres://user:pa@ss@host/db",
    "postgres://REDACTED@host/db"
)]
#[case::db_conn_loses_to_the_earlier_ranked_generic_api_key(
    "redis://api_key:abcdefghij@host/db",
    "redis://REDACTED"
)]
#[case::db_conn_loses_to_the_earlier_ranked_generic_password(
    "postgres://db_password=xyz:pw@host",
    "postgres://REDACTED"
)]
#[case::db_conn_keeps_the_stronger_redaction_over_a_weaker_earlier_ranked_sk_key(
    "postgres://sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:hunter2pass@host/db",
    "postgres://REDACTED@host/db"
)]
#[case::url_query_cred_keeps_the_stronger_redaction_over_a_weaker_earlier_ranked_akia_key(
    "GET /x?AKIA0123456789ABCDEFtoken=leakedvalue HTTP/1.1",
    "GET /x?REDACTED HTTP/1.1"
)]
#[case::malformed_pem_block_without_an_end_marker_is_not_redacted(
    &format!("cert=-----BEGIN RSA PRIVATE KEY-----\n{}", "M".repeat(200_000)),
    &format!("cert=-----BEGIN RSA PRIVATE KEY-----\n{}", "M".repeat(200_000))
)]
#[case::malformed_db_conn_credential_without_a_closing_at_sign_is_not_redacted(
    &format!("postgres://user:{}", "p".repeat(200_000)),
    &format!("postgres://user:{}", "p".repeat(200_000))
)]
#[case::large_near_match_without_a_delimiter_before_the_token_suffix_is_not_redacted(
    &format!("{}_token=x", "a".repeat(200_000)),
    &format!("{}_token=x", "a".repeat(200_000))
)]
#[case::large_near_match_without_a_scheme_before_the_colon_suffix_is_not_redacted(
    &format!("{}:x", "a".repeat(200_000)),
    &format!("{}:x", "a".repeat(200_000))
)]
#[case::large_near_match_with_a_real_secret_redacts_without_exposing_the_supplied_sentinel(
    &format!("{}_token=x&token=SUPPLIED-SECRET-SENTINEL", "a".repeat(200_000)),
    &format!("{}_token=x&REDACTED", "a".repeat(200_000))
)]
#[case::url_query_cred_no_category_digit_prefix_is_redacted(
    "?\u{bd}token=abcdefghijklmnop",
    "?REDACTED"
)]
#[case::bare_password_no_category_digit_prefix_is_redacted(
    "my\u{bd}password=hunter2hunter2",
    "REDACTED"
)]
#[case::combining_mark_before_ya29_token_is_redacted(
    "\u{301}ya29.abcdefghij1234567890",
    "\u{301}REDACTED"
)]
#[case::combining_mark_before_jwt_token_is_redacted(
    "\u{301}eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj",
    "\u{301}REDACTED"
)]
#[case::dotless_i_in_api_key_is_redacted("ap\u{131}_key=abcdefghijk", "REDACTED")]
#[case::dotted_i_in_akia_access_key_is_redacted("AK\u{130}A0123456789ABCDEF", "REDACTED")]
#[case::dotless_i_inside_a_ya29_token_is_redacted("ya29.abcd\u{131}efghijSECRET", "REDACTED")]
#[case::dotless_i_inside_an_sk_key_is_redacted(
    "sk-abcdefghijklmnop\u{131}qrstuvwxyzSECRET",
    "REDACTED"
)]
#[case::jwt_after_a_bearer_token_ending_in_equals_redacts_both(
    "Bearer abcdefghijk=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj",
    "REDACTEDREDACTED"
)]
#[case::ya29_after_a_basic_token_ending_in_equals_redacts_both(
    "Basic YWJjZGVmZ2hpams=ya29.abcdefghij1234567890",
    "REDACTEDREDACTED"
)]
#[case::service_account_object_wins_the_brace_before_a_raw_jwt(
    r#"{eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj "type": "service_account", "client_email": "a@b.c"}"#,
    "REDACTED"
)]
#[case::jwt_preceded_by_a_word_character_is_not_a_boundary_match(
    "fooeyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj",
    "fooeyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBj"
)]
#[case::ya29_preceded_by_a_word_character_is_not_a_boundary_match(
    "fooya29.abcdefghij1234567890",
    "fooya29.abcdefghij1234567890"
)]
fn secret_redaction_redaction_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let started = Instant::now();
    let output = redactor.redact_string(input);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    // The equality check below already proves the supplied sentinel never survives into output.
    assert_eq!(output, expected);
}

#[test]
fn google_aiza_bare_key_is_redacted() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = format!("AIza{}", "A".repeat(35));
    assert_eq!(redactor.redact_string(&text), "REDACTED");
}
