#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Raw, Settings};
use crate::loader::parse_view;
use crate::sections::Section as _;
use crate::sections::settings::JsonValue;

fn load(source: &str) -> Settings {
    let (raw, _ignored): (Raw, _) = parse_view(source).expect("valid litellm_settings yaml");
    raw.convert(&BTreeMap::<String, String>::new())
        .expect("convert never fails for this section")
}

#[rstest]
#[case::absent("{}\n", "annotate")]
#[case::explicit_error(
    "litellm_settings:\n  vector_store_search_failure_mode: error\n",
    "error"
)]
#[case::invalid_bogus(
    "litellm_settings:\n  vector_store_search_failure_mode: bogus\n",
    "bogus"
)]
fn loads_failure_mode(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        load(source).failure_mode,
        JsonValue::String(expected.to_owned())
    );
}
