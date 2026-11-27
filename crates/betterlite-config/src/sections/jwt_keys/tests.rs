#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

use super::PUBLIC_KEY_URL_VARIABLE;

fn loaded(environment: &[(&str, &str)]) -> super::Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("model_list: []\n").unwrap();
    config.sections.jwt_keys
}

#[test]
fn jwt_public_url_absent_empty_and_comma_list() {
    assert_eq!(loaded(&[]).public_key_url, None);
    assert_eq!(
        loaded(&[(PUBLIC_KEY_URL_VARIABLE, "")]).public_key_url,
        Some(String::new())
    );
    let listed = " https://a.example.com/keys , https://b.example.com/keys ";
    assert_eq!(
        loaded(&[(PUBLIC_KEY_URL_VARIABLE, listed)]).public_key_url,
        Some(listed.to_owned())
    );
}
