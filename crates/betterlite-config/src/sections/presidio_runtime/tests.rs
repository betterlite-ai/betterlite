#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::presidio_runtime::Settings;

fn settings(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.presidio_runtime)
}

#[rstest]
#[case::absent("model_list: []\n", false)]
#[case::empty_settings("litellm_settings: {}\n", false)]
#[case::enabled("litellm_settings:\n  output_parse_pii: true\n", true)]
#[case::disabled("litellm_settings:\n  output_parse_pii: false\n", false)]
#[case::null("litellm_settings:\n  output_parse_pii: null\n", false)]
#[case::beside_other_keys(
    "litellm_settings:\n  disable_aiohttp_transport: true\n  output_parse_pii: true\n",
    true
)]
fn loads_output_parse_pii(#[case] source: &str, #[case] expected: bool) {
    let loaded = settings(source).expect("the section");
    assert_eq!(loaded.output_parse_pii, expected);
}

#[test]
fn rejects_a_wrong_type() {
    let error = settings("litellm_settings:\n  output_parse_pii: yes please\n")
        .expect_err("a string is not a flag");
    let text = error.to_string();
    assert!(text.contains("litellm_settings.output_parse_pii"));
    let ConfigError::InvalidScalar { path, .. } = &error else {
        panic!("expected an invalid scalar");
    };
    assert_eq!(path, "litellm_settings.output_parse_pii");
}
