#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;

fn loaded(environment: &[(&str, &str)]) -> (Option<String>, Option<String>) {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads");
    let settings = &config.sections.openai_like_settings;
    let key = settings
        .api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    (settings.api_base.clone(), key)
}

#[rstest]
#[case::both(&[("OPENAI_LIKE_API_BASE", "https://env.test/v1"), ("OPENAI_LIKE_API_KEY", "sk-env")], Some("https://env.test/v1"), Some("sk-env"))]
#[case::base_only(&[("OPENAI_LIKE_API_BASE", "https://env.test/v1")], Some("https://env.test/v1"), None)]
#[case::key_only(&[("OPENAI_LIKE_API_KEY", "sk-env")], None, Some("sk-env"))]
#[case::empty_values_are_unset(&[("OPENAI_LIKE_API_BASE", ""), ("OPENAI_LIKE_API_KEY", "")], None, None)]
#[case::absent(&[], None, None)]
fn loads_compatible_provider_settings(
    #[case] environment: &[(&str, &str)],
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
) {
    let (loaded_base, loaded_key) = loaded(environment);
    assert_eq!(loaded_base.as_deref(), base);
    assert_eq!(loaded_key.as_deref(), key);
}
