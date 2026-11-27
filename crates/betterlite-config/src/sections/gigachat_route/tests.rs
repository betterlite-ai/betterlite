#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::gigachat_route::Settings;

fn settings_from(environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty document still loads")
        .sections
        .gigachat_route
}

#[rstest]
#[case::url_is_kept(
    &[("GIGACHAT_API_BASE", "https://gigachat.example/api/v1")],
    Some("https://gigachat.example/api/v1")
)]
#[case::unset(&[], None)]
#[case::empty_value(&[("GIGACHAT_API_BASE", "")], None)]
#[case::true_text(&[("GIGACHAT_API_BASE", "true")], None)]
#[case::padded_false_text(&[("GIGACHAT_API_BASE", " False ")], None)]
#[case::other_text_is_kept(&[("GIGACHAT_API_BASE", "yes")], Some("yes"))]
#[case::trailing_space_is_kept(
    &[("GIGACHAT_API_BASE", "https://gigachat.example/api/v1 ")],
    Some("https://gigachat.example/api/v1 ")
)]
fn reads_the_environment(#[case] environment: &[(&str, &str)], #[case] expected: Option<&str>) {
    let settings = settings_from(environment);
    assert_eq!(settings.api_base.as_deref(), expected);
}

#[test]
fn the_base_url_variable_does_not_fill_the_field() {
    let settings = settings_from(&[("GIGACHAT_BASE_URL", "https://gigachat.example/api/v1")]);
    assert!(settings.api_base.is_none());
}
