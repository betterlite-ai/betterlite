#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(yaml: &str) -> Result<Option<i64>, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(yaml)
    .map(|config| config.sections.langsmith_settings.batch_size)
}

#[rstest]
#[case::absent("{}", None)]
#[case::null("litellm_settings: {langsmith_batch_size: null}", None)]
#[case::configured("litellm_settings: {langsmith_batch_size: 64}", Some(64))]
#[case::explicit_zero("litellm_settings: {langsmith_batch_size: 0}", Some(0))]
#[case::negative("litellm_settings: {langsmith_batch_size: -1}", Some(-1))]
#[case::quoted_digits("litellm_settings: {langsmith_batch_size: \"64\"}", Some(64))]
fn langsmith_batch_size_loads(#[case] yaml: &str, #[case] expected: Option<i64>) {
    assert_eq!(load(yaml).expect("the config loads"), expected);
}

#[rstest]
#[case::word("litellm_settings: {langsmith_batch_size: many}")]
#[case::fractional("litellm_settings: {langsmith_batch_size: 1.5}")]
#[case::boolean("litellm_settings: {langsmith_batch_size: true}")]
fn langsmith_batch_size_refuses_wrong_types(#[case] yaml: &str) {
    let _error = load(yaml).expect_err("a value of the wrong type fails the load");
}
