#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::*;

#[rstest]
#[case::phrase("rate limit reached", None, true)]
#[case::number_with_429("429", Some(429), true)]
#[case::number_with_other_status("429", Some(400), false)]
#[case::number_inside_word("429abc", None, false)]
#[case::number_inside_long_number("1429", None, false)]
#[case::number_after_underscore("x_429", None, false)]
#[case::number_before_underscore("429_", None, false)]
#[case::service_tier("service tier capacity exceeded", Some(400), true)]
#[case::hyphenated("rate-limit reached", Some(400), true)]
#[case::ordinary("bad request", None, false)]
#[case::repeated_separators("rate--limit", None, true)]
#[case::no_separator("ratelimit", None, true)]
#[case::tab_separator("rate\tlimit", None, true)]
#[case::unicode_word_before("\u{e9}429", None, false)]
#[case::unicode_word_after("429\u{e9}", None, false)]
fn error_facts_behavior_tests_behavior(
    #[case] message: &str,
    #[case] status: Option<u16>,
    #[case] expected: bool,
) {
    assert_eq!(
        ExceptionCheckers::is_error_str_rate_limit(message, status),
        expected
    );
}

#[path = "tests/context_window/tests.rs"]
mod context_window;

#[path = "tests/azure_policy/tests.rs"]
mod azure_policy;

#[path = "tests/proxy_reconstruction/tests.rs"]
mod proxy_reconstruction;

#[path = "tests/proxy_empty_headers/tests.rs"]
mod proxy_empty_headers;

#[path = "tests/proxy_fallback/tests.rs"]
mod proxy_fallback;

#[path = "tests/body_boundaries/tests.rs"]
mod body_boundaries;

#[path = "tests/absent_response/tests.rs"]
mod absent_response;

mod azure_policy_gate_cases;

mod proxy_header_gate_cases;
