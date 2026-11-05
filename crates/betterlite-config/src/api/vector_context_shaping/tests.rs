#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::json;

use crate::loader::ConfigLoader;
use crate::sections::settings::JsonValue;

#[rstest]
#[case::absent("{}\n", json!("annotate"), BTreeMap::new())]
#[case::explicit_error(
    "litellm_settings:\n  vector_store_search_failure_mode: error\n",
    json!("error"),
    BTreeMap::new()
)]
#[case::invalid_bogus(
    "litellm_settings:\n  vector_store_search_failure_mode: bogus\n",
    json!("bogus"),
    BTreeMap::new()
)]
#[case::environment(
    "litellm_settings:\n  vector_store_search_failure_mode: os.environ/VECTOR_STORE_MODE\n",
    json!("error"),
    BTreeMap::from([("VECTOR_STORE_MODE".to_owned(), "error".to_owned())])
)]
#[case::invalid_number(
    "litellm_settings:\n  vector_store_search_failure_mode: 1\n",
    json!(1),
    BTreeMap::new()
)]
#[case::invalid_null(
    "litellm_settings:\n  vector_store_search_failure_mode: null\n",
    JsonValue::Null,
    BTreeMap::new()
)]
fn loads_failure_mode(
    #[case] source: &str,
    #[case] expected: JsonValue,
    #[case] lookup: BTreeMap<String, String>,
) {
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("failure mode is a supported configuration key");
    assert_eq!(
        config.sections.vector_context_shaping.failure_mode,
        expected
    );
}
