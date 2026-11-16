#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://fal.run";

fn config_loader(environment: &[(&str, &str)]) -> ConfigLoader<BTreeMap<String, String>> {
    ConfigLoader {
        lookup: environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn load(environment: &[(&str, &str)]) -> (Option<String>, String) {
    let loader = config_loader(environment);
    let config = loader.load("").expect("an empty config loads");
    assert!(config.model_list.is_empty());
    let (api_key, api_base) = Sections::fal_image_environment(&loader.lookup);
    (
        api_key.map(|secret| secret.expose_secret().to_owned()),
        api_base,
    )
}

fn queue_settings(environment: &[(&str, &str)]) -> (Option<String>, Option<String>) {
    let loader = config_loader(environment);
    let (api_key, queue_base) = Sections::fal_queue_settings(&loader.lookup);
    (
        api_key.map(|secret| secret.expose_secret().to_owned()),
        queue_base,
    )
}

#[rstest]
#[case::defaults(&[], (None, DEFAULT_API_BASE))]
#[case::configured(
    &[("FAL_AI_API_KEY", "fal-key"), ("FAL_AI_API_BASE", "https://proxy.test/fal")],
    (Some("fal-key"), "https://proxy.test/fal")
)]
#[case::key_only(&[("FAL_AI_API_KEY", "fal-key")], (Some("fal-key"), DEFAULT_API_BASE))]
#[case::base_only(&[("FAL_AI_API_BASE", "https://proxy.test/fal")], (None, "https://proxy.test/fal"))]
#[case::empty_key(&[("FAL_AI_API_KEY", "")], (None, DEFAULT_API_BASE))]
#[case::empty_base(&[("FAL_AI_API_BASE", "")], (None, DEFAULT_API_BASE))]
#[case::both_empty(
    &[("FAL_AI_API_KEY", ""), ("FAL_AI_API_BASE", "")],
    (None, DEFAULT_API_BASE)
)]
fn fal_image_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, &str),
) {
    let (api_key, api_base) = load(environment);
    assert_eq!(api_key.as_deref(), expected.0);
    assert_eq!(api_base, expected.1);
}

#[rstest]
#[case::defaults(&[], (None, None))]
#[case::configured(
    &[("FAL_AI_API_KEY", "fal-key"), ("FAL_AI_QUEUE_API_BASE", "https://queue.test/fal")],
    (Some("fal-key"), Some("https://queue.test/fal"))
)]
#[case::key_only(&[("FAL_AI_API_KEY", "fal-key")], (Some("fal-key"), None))]
#[case::queue_base_only(
    &[("FAL_AI_QUEUE_API_BASE", "https://queue.test/fal")],
    (None, Some("https://queue.test/fal"))
)]
#[case::empty_key(&[("FAL_AI_API_KEY", "")], (None, None))]
#[case::empty_queue_base(&[("FAL_AI_QUEUE_API_BASE", "")], (None, None))]
fn fal_queue_settings(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, Option<&str>),
) {
    let (api_key, queue_base) = queue_settings(environment);
    assert_eq!((api_key.as_deref(), queue_base.as_deref()), expected);
}
