#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::types::RoutingStrategy;

const DEFAULT_API_BASE: &str = "https://external.api.recraft.ai";

fn settings(entries: &[(&str, &str)]) -> (Option<String>, String) {
    let lookup: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loader = ConfigLoader { lookup };
    let config = loader.load("").expect("the empty config loads");
    assert!(config.model_list.is_empty());
    let (api_key, api_base) = RoutingStrategy::recraft_image_settings(&loader.lookup);
    (api_key.map(|key| key.expose_secret().to_owned()), api_base)
}

#[rstest]
#[case::unset(&[], (None, DEFAULT_API_BASE))]
#[case::read_the_environment(
    &[("RECRAFT_API_KEY", "server-key"), ("RECRAFT_API_BASE", "https://proxy.test/recraft")],
    (Some("server-key"), "https://proxy.test/recraft")
)]
#[case::key_only(&[("RECRAFT_API_KEY", "server-key")], (Some("server-key"), DEFAULT_API_BASE))]
#[case::base_only(
    &[("RECRAFT_API_BASE", "https://proxy.test/recraft")],
    (None, "https://proxy.test/recraft")
)]
#[case::empty_variables_are_unset(
    &[("RECRAFT_API_KEY", ""), ("RECRAFT_API_BASE", "")],
    (None, DEFAULT_API_BASE)
)]
#[case::boolean_text_is_unset(
    &[("RECRAFT_API_KEY", "true"), ("RECRAFT_API_BASE", "FALSE")],
    (None, DEFAULT_API_BASE)
)]
fn recraft_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, &str),
) {
    let (api_key, api_base) = settings(environment);

    assert_eq!(api_key.as_deref(), expected.0);
    assert_eq!(api_base, expected.1);
}
