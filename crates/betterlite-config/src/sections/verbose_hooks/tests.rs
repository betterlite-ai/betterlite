#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;

#[rstest]
#[case::loads_set_verbose("litellm_settings:\n  set_verbose: true\n", BTreeMap::new(), true)]
#[case::default("{}\n", BTreeMap::new(), false)]
fn verbose_hooks_config_cases(
    #[case] source: &str,
    #[case] lookup: BTreeMap<String, String>,
    #[case] expected: bool,
) {
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("config should load");
    assert_eq!(config.sections.verbose_hooks.set_verbose, expected);
}
