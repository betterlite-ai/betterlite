#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::cometapi_embedding_credentials::Settings;
use rstest::rstest;
use secrecy::ExposeSecret;

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .cometapi_embedding_credentials
}

fn revealed(settings: &Settings) -> Option<&str> {
    settings.api_key.as_ref().map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::missing(&[], None)]
#[case::explicit(&[("COMETAPI_KEY", "primary")], Some("primary"))]
#[case::empty(&[("COMETAPI_KEY", "")], None)]
#[case::alternative_alias_only(&[("COMETAPI_API_KEY", "alternative")], None)]
#[case::primary_alongside_the_alias(
    &[("COMETAPI_KEY", "primary"), ("COMETAPI_API_KEY", "alternative")],
    Some("primary")
)]
fn only_primary_key_alias(#[case] environment: &[(&str, &str)], #[case] expected: Option<&str>) {
    assert_eq!(revealed(&settings(environment)), expected);
}
