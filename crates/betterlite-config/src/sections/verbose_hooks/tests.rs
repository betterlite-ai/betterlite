#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::json;

use super::SettingValue;

use crate::loader::ConfigLoader;

#[rstest]
#[case::loads_set_verbose("litellm_settings:\n  set_verbose: true\n", BTreeMap::new(), json!(true))]
#[case::loads_truthy_non_boolean("litellm_settings:\n  set_verbose: 1\n", BTreeMap::new(), json!(1_i64))]
#[case::default("{}\n", BTreeMap::new(), json!(false))]
fn verbose_hooks_config_cases(
    #[case] source: &str,
    #[case] lookup: BTreeMap<String, String>,
    #[case] expected: SettingValue,
) {
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("config should load");
    assert_eq!(config.sections.verbose_hooks.set_verbose, expected);
}
