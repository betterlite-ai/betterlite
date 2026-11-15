#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{
    DEFAULT_PROMPT_ENVIRONMENT, default_serve_environment, get_base_prompt_id, get_version_number,
    parse_prompt_version, prompt_environment_or_default, registry_key_for_prompt, spec_version,
};
use crate::tests::{json_value, prompt_spec};

#[rstest]
#[case::dot_marker("jack_success.v1", "jack_success")]
#[case::underscore_marker("jack_success_v1", "jack_success")]
#[case::no_marker("jack_success", "jack_success")]
#[case::unparsable_dot_marker("jack_success.vabc", "jack_success")]
#[case::unparsable_underscore_marker("jack_success_vabc", "jack_success")]
#[case::dot_marker_wins_over_underscore("jack_v1.v2", "jack_v1")]
#[case::dot_marker_splits_at_the_first_one("jack.v1.v2", "jack")]
fn get_base_prompt_id_strips_one_version_marker(#[case] prompt_id: &str, #[case] expected: &str) {
    assert_eq!(get_base_prompt_id(prompt_id), expected);
}

#[rstest]
#[case::dot_marker("jack_success.v2", 2)]
#[case::underscore_marker("jack_success_v2", 2)]
#[case::no_marker("jack_success", 1)]
#[case::unparsable_dot_marker("jack_success.vtwo", 1)]
#[case::underscore_marker_after_unparsable_dot("jack_success.vtwo_v3", 3)]
#[case::dot_marker_wins_over_underscore("jack_success.v2.v3", 2)]
#[case::leading_zero("jack_success.v02", 2)]
#[case::empty_dot_suffix("jack_success.v", 1)]
#[case::negative_number("jack_success.v-2", -2)]
#[case::leading_plus("jack_success.v+2", 2)]
#[case::space_before_the_number("jack_success.v 2", 1)]
#[case::underscore_inside_the_number("jack_success.v2_0", 1)]
#[case::unicode_digit("jack_success.v\u{0662}", 1)]
#[case::version_beyond_i64("jack_success.v9223372036854775808", 1)]
fn get_version_number_reads_the_suffix_or_defaults(#[case] prompt_id: &str, #[case] expected: i64) {
    assert_eq!(get_version_number(prompt_id), expected);
}

#[rstest]
#[case::integer(json_value("3"), Some(3))]
#[case::negative_integer(json_value("-1"), Some(-1))]
#[case::integer_beyond_i64(json_value("9223372036854775808"), None)]
#[case::digit_string(json_value("\"12\""), Some(12))]
#[case::leading_zero_string(json_value("\"012\""), Some(12))]
#[case::empty_string(json_value("\"\""), None)]
#[case::word_string(json_value("\"abc\""), None)]
#[case::signed_string(json_value("\"-1\""), None)]
#[case::unicode_digit_string(json_value("\"\u{0662}\""), None)]
#[case::superscript_digit_string(json_value("\"\u{00B2}\""), None)]
#[case::decimal_string(json_value("\"2.5\""), None)]
#[case::float(json_value("2.5"), None)]
#[case::boolean(json_value("true"), None)]
#[case::null(json_value("null"), None)]
#[case::list(json_value("[1]"), None)]
#[case::object(json_value("{\"version\":1}"), None)]
fn parse_prompt_version_accepts_integers_and_digit_strings(
    #[case] raw_version: Json,
    #[case] expected: Option<i64>,
) {
    assert_eq!(parse_prompt_version(Some(&raw_version)), expected);
}

#[test]
fn parse_prompt_version_returns_none_without_a_stored_version() {
    assert_eq!(parse_prompt_version(None), None);
}

#[rstest]
#[case::absent(None, "development")]
#[case::explicit(Some("production"), "production")]
#[case::empty(Some(""), "")]
fn prompt_environment_or_default_only_defaults_the_absent_environment(
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(prompt_environment_or_default(environment), expected);
}

#[test]
fn the_default_environment_matches_the_litellm_default() {
    assert_eq!(DEFAULT_PROMPT_ENVIRONMENT, "development");
}

#[test]
fn registry_key_for_prompt_combines_the_prompt_id_and_environment() {
    let prompt = prompt_spec("chat.v2", Some(2), "production");
    assert_eq!(registry_key_for_prompt(&prompt), "chat.v2::production");
}

#[test]
fn registry_key_for_prompt_keeps_an_empty_environment() {
    let prompt = prompt_spec("chat.v2", Some(2), "");
    assert_eq!(registry_key_for_prompt(&prompt), "chat.v2::");
}

#[test]
fn spec_version_prefers_the_explicit_version_field() {
    let prompt = prompt_spec("chat.v1", Some(5), "development");
    assert_eq!(spec_version(&prompt), 5);
}

#[test]
fn spec_version_falls_back_to_the_id_suffix() {
    let prompt = prompt_spec("chat.v4", None, "development");
    assert_eq!(spec_version(&prompt), 4);
}

#[test]
fn spec_version_defaults_to_one_without_a_field_or_suffix() {
    let prompt = prompt_spec("chat", None, "development");
    assert_eq!(spec_version(&prompt), 1);
}

#[test]
fn default_serve_environment_prefers_production_over_later_environments() {
    let development = prompt_spec("chat.v1", None, "development");
    let staging = prompt_spec("chat.v1", None, "staging");
    let production = prompt_spec("chat.v1", None, "production");
    assert_eq!(
        default_serve_environment(&[&development, &staging, &production]),
        "production"
    );
}

#[test]
fn default_serve_environment_prefers_staging_over_development() {
    let development = prompt_spec("chat.v1", None, "development");
    let staging = prompt_spec("chat.v1", None, "staging");
    assert_eq!(
        default_serve_environment(&[&development, &staging]),
        "staging"
    );
}

#[test]
fn default_serve_environment_falls_back_to_development_when_present() {
    let development = prompt_spec("chat.v1", None, "development");
    let beta = prompt_spec("chat.v1", None, "beta");
    assert_eq!(
        default_serve_environment(&[&beta, &development]),
        "development"
    );
}

#[test]
fn default_serve_environment_picks_the_alphabetically_first_other_environment() {
    let beta = prompt_spec("chat.v1", None, "beta");
    let alpha = prompt_spec("chat.v1", None, "alpha");
    assert_eq!(default_serve_environment(&[&beta, &alpha]), "alpha");
}

#[test]
fn default_serve_environment_is_development_without_prompts() {
    assert_eq!(default_serve_environment(&[]), DEFAULT_PROMPT_ENVIRONMENT);
}
