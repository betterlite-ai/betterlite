#![cfg(test)]

use super::scrub;
use rstest::rstest;
use std::str::from_utf8;
use std::{fs, path::PathBuf};

#[rstest]
#[case::denies_field_with_boundary("api_key='abc123'", false, "api_key='[Filtered]'")]
#[case::rejects_name_prefix_match("mytoken=abc123", false, "mytoken=abc123")]
#[case::matches_at_string_start("token=abc123 next", false, "token='[Filtered]' next")]
#[case::preserves_bare_none("token=None", false, "token=None")]
#[case::redacts_nonetype_suffix("key=NoneType", false, "key='[Filtered]'")]
#[case::quoted_key_form_skips_boundary_check(
    "mypassword': 'hunter2', 'password': 'hunter2'",
    false,
    "mypassword': 'hunter2', 'password': '[Filtered]'"
)]
#[case::pii_field_redacted_by_default("user_email='x@y.com'", false, "user_email='[Filtered]'")]
#[case::pii_field_kept_when_sending_pii("user_email='x@y.com'", true, "user_email='x@y.com'")]
#[case::key_pattern_always_substituted(
    "key is sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa here",
    true,
    "key is [Filtered] here"
)]
#[case::email_kept_when_sending_pii("contact user@example.com", true, "contact user@example.com")]
#[case::email_filtered_without_pii("contact user@example.com", false, "contact [Filtered]")]
#[case::sha256_blocked_by_word_boundary(
    "xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdy",
    false,
    "xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdy"
)]
#[case::sha256_filtered_when_isolated(
    "hash: abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd end",
    false,
    "hash: [Filtered] end"
)]
#[case::bracketed_value_depth_four_fully_filtered(
    "password=[[[[x]]]]",
    false,
    "password='[Filtered]'"
)]
fn sentry_scrubbing_behavior_tests_behavior(
    #[case] input: &str,
    #[case] send_default_pii: bool,
    #[case] expected: &str,
) {
    assert_eq!(scrub(input, send_default_pii).unwrap(), expected);
}

#[test]
fn bracketed_value_beyond_supported_depth_falls_back_to_bare_value() {
    let output = scrub("password=[[[[[x]]]]]", false).unwrap();
    assert!(output.starts_with("password='[Filtered]'"));
    assert_ne!(output, "password='[Filtered]'");
}

#[rstest]
fn replays_the_sentry_scrubbing_corpus(#[files("fuzz/corpus/sentry_scrubbing/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    drop(scrub(text, false));
    drop(scrub(text, true));
}
