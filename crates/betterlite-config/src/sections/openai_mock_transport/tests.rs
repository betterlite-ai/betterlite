#![cfg(test)]

use crate::loader::ConfigLoader;
use std::collections::BTreeMap;

#[rstest::rstest]
#[case::loads_network_mock("", None, false)]
#[case::loads_network_mock_explicit_false("litellm_settings:\n  network_mock: false\n", None, false)]
#[case::loads_network_mock_explicit_true("litellm_settings:\n  network_mock: true\n", None, true)]
fn openai_mock_transport_config_cases(
    #[case] source: &str,
    #[case] environment: Option<&str>,
    #[case] expected: bool,
) {
    let lookup = environment.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([(String::from("LITELLM_NETWORK_MOCK"), value.to_owned())])
    });
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("the openai mock transport settings are valid");
    assert_eq!(config.sections.openai_mock_transport.network_mock, expected);
}
