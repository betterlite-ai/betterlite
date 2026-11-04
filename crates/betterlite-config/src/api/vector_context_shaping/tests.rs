#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::settings::JsonValue;

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
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("failure mode is a supported configuration key");
    assert_eq!(
        config.sections.vector_context_shaping.failure_mode,
        JsonValue::String(expected.to_owned())
    );
}
