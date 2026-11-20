#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::provider_budget_view::ProviderBudgetConfig;

const DEFAULT_API_BASE: &str = "https://api.topazlabs.com";

fn settings(entries: &[(&str, &str)]) -> (Option<String>, String) {
    let lookup: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loader = ConfigLoader { lookup };
    let config = loader.load("").expect("the empty config loads");
    assert!(config.model_list.is_empty());
    let (api_key, api_base) = ProviderBudgetConfig::topaz_image_settings(&loader.lookup);
    (api_key.map(|key| key.expose_secret().to_owned()), api_base)
}

#[rstest]
#[case::unset(&[], (None, DEFAULT_API_BASE))]
#[case::read_the_environment(
    &[("TOPAZ_API_KEY", "server-key"), ("TOPAZ_API_BASE", "https://proxy.test/v1")],
    (Some("server-key"), "https://proxy.test/v1")
)]
#[case::key_only(&[("TOPAZ_API_KEY", "server-key")], (Some("server-key"), DEFAULT_API_BASE))]
#[case::base_only(
    &[("TOPAZ_API_BASE", "https://proxy.test/v1")],
    (None, "https://proxy.test/v1")
)]
#[case::empty_variables_are_unset(
    &[("TOPAZ_API_KEY", ""), ("TOPAZ_API_BASE", "")],
    (None, DEFAULT_API_BASE)
)]
#[case::boolean_text_is_unset(
    &[("TOPAZ_API_KEY", "true"), ("TOPAZ_API_BASE", "FALSE")],
    (None, DEFAULT_API_BASE)
)]
fn topaz_settings(#[case] environment: &[(&str, &str)], #[case] expected: (Option<&str>, &str)) {
    let (api_key, api_base) = settings(environment);

    assert_eq!(api_key.as_deref(), expected.0);
    assert_eq!(api_base, expected.1);
}
