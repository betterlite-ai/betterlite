#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://fal.run";

fn load(environment: &[(&str, &str)]) -> (Option<String>, String) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loader = ConfigLoader { lookup };
    let config = loader.load("").expect("an empty config loads");
    assert!(config.model_list.is_empty());
    let (api_key, api_base) = Sections::fal_image_environment(&loader.lookup);
    (
        api_key.map(|secret| secret.expose_secret().to_owned()),
        api_base,
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
