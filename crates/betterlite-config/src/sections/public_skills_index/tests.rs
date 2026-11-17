#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<super::Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .public_skills_index)
}

#[rstest]
#[case::no_settings("", false)]
#[case::empty_settings("litellm_settings: {}\n", false)]
#[case::true_flag("litellm_settings:\n  public_skills_index: true\n", true)]
#[case::false_flag("litellm_settings:\n  public_skills_index: false\n", false)]
#[case::null_flag("litellm_settings:\n  public_skills_index:\n", false)]
fn reads_the_public_skills_index_flag(#[case] source: &str, #[case] expected: bool) {
    let settings = load(source).unwrap();
    assert_eq!(settings.public_skills_index, expected);
}
