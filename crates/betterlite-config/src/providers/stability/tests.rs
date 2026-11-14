#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::loader::ConfigLoader;
use crate::types::Config;

const DEFAULT_API_BASE: &str = "https://api.stability.ai";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(entries: &[(&str, &str)]) -> (Option<String>, String) {
    let loader = ConfigLoader {
        lookup: lookup(entries),
    };
    let loaded = loader.load("").expect("the empty config loads");
    assert!(loaded.model_list.is_empty());
    let (api_key, api_base) = Config::stability_generation_settings(&loader.lookup);
    (api_key.map(|key| key.expose_secret().to_owned()), api_base)
}

#[rstest]
#[case::unset(&[], None, DEFAULT_API_BASE)]
#[case::read_the_environment(
    &[
        ("STABILITY_API_KEY", "server-key"),
        ("STABILITY_API_BASE", "https://proxy.test/v1"),
    ],
    Some("server-key"),
    "https://proxy.test/v1"
)]
#[case::empty_variables_are_unset(
    &[("STABILITY_API_KEY", ""), ("STABILITY_API_BASE", "")],
    None,
    DEFAULT_API_BASE
)]
fn stability_generation_environment(
    #[case] entries: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] api_base: &str,
) {
    let (found_key, found_base) = settings(entries);

    assert_eq!(found_key.as_deref(), api_key);
    assert_eq!(found_base, api_base);
}
