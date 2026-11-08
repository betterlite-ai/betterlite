#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::proxy_base_url::Settings;

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("{}")?.sections.proxy_base_url)
}

#[test]
fn proxy_base_url_unset_is_none() {
    let settings = load(&[]).unwrap();
    assert_eq!(settings.proxy_base_url, None);
}

#[test]
fn proxy_base_url_reads_the_environment_exactly() {
    let settings = load(&[("PROXY_BASE_URL", "https://api.example.com/base/")]).unwrap();
    assert_eq!(
        settings.proxy_base_url.as_deref(),
        Some("https://api.example.com/base/")
    );
}

#[test]
fn proxy_base_url_set_but_empty_stays_some_empty() {
    let settings = load(&[("PROXY_BASE_URL", "")]).unwrap();
    assert_eq!(settings.proxy_base_url.as_deref(), Some(""));
}
