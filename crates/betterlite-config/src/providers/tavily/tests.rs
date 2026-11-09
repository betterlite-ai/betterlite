#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::loader::EnvironmentLookup;

const TAVILY_API_BASE: &str = "https://api.tavily.com";

struct TavilySearchSettings {
    api_key: Option<SecretString>,
    api_base: String,
}

impl TavilySearchSettings {
    fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present("TAVILY_API_KEY").map(SecretString::from),
            api_base: present("TAVILY_API_BASE").unwrap_or_else(|| TAVILY_API_BASE.to_owned()),
        }
    }
}

fn load(environment: &[(&str, &str)]) -> TavilySearchSettings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    TavilySearchSettings::load(&lookup)
}

#[test]
fn tavily_search_settings_default_to_the_published_base() {
    let settings = load(&[]);
    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, "https://api.tavily.com");
    assert_eq!(settings.api_base, TAVILY_API_BASE);
}

#[rstest]
#[case::key_and_base(
    &[("TAVILY_API_KEY", "sk-test"), ("TAVILY_API_BASE", "https://proxy.test")],
    Some("sk-test"),
    "https://proxy.test"
)]
#[case::empty_key_is_unset(&[("TAVILY_API_KEY", "")], None, TAVILY_API_BASE)]
#[case::empty_base_is_the_default(&[("TAVILY_API_BASE", "")], None, TAVILY_API_BASE)]
#[case::unrelated_variables_are_ignored(&[("OTHER", "x")], None, TAVILY_API_BASE)]
fn tavily_search_settings_read_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let settings = load(environment);
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(settings.api_base, expected_base);
}
