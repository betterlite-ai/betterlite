#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Raw, Settings};
use crate::loader::parse_view;
use crate::sections::Section as _;
use crate::sections::settings::JsonValue;

fn load(source: &str, lookup: BTreeMap<String, String>) -> Settings {
    let (raw, _ignored): (Raw, _) = parse_view(source).expect("valid litellm_settings yaml");
    raw.convert(&lookup)
        .expect("convert never fails for this section")
}

#[rstest]
#[case::absent("{}\n", "annotate", BTreeMap::new())]
#[case::explicit_error(
    "litellm_settings:\n  vector_store_search_failure_mode: error\n",
    "error",
    BTreeMap::new()
)]
#[case::invalid_bogus(
    "litellm_settings:\n  vector_store_search_failure_mode: bogus\n",
    "bogus",
    BTreeMap::new()
)]
#[case::environment(
    "litellm_settings:\n  vector_store_search_failure_mode: os.environ/VECTOR_STORE_MODE\n",
    "error",
    BTreeMap::from([("VECTOR_STORE_MODE".to_owned(), "error".to_owned())])
)]
fn loads_failure_mode(
    #[case] source: &str,
    #[case] expected: &str,
    #[case] lookup: BTreeMap<String, String>,
) {
    assert_eq!(
        load(source, lookup).failure_mode,
        JsonValue::String(expected.to_owned())
    );
}
