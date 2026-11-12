#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::database_tokens::settings::{POOLED_MARKER_ENV_VAR, database_url_is_pooled};

#[rstest]
#[case::exact_true(POOLED_MARKER_ENV_VAR, "true", true)]
#[case::title_case(POOLED_MARKER_ENV_VAR, "True", false)]
#[case::upper_case(POOLED_MARKER_ENV_VAR, "TRUE", false)]
#[case::one(POOLED_MARKER_ENV_VAR, "1", false)]
#[case::yes(POOLED_MARKER_ENV_VAR, "yes", false)]
#[case::empty(POOLED_MARKER_ENV_VAR, "", false)]
#[case::absent("UNRELATED_VARIABLE", "true", false)]
fn pooled_marker_exact_true_only(#[case] name: &str, #[case] value: &str, #[case] expected: bool) {
    let environment = BTreeMap::from([(name.to_owned(), value.to_owned())]);
    assert_eq!(
        database_url_is_pooled(&environment),
        expected,
        "{name}={value:?}"
    );
}
