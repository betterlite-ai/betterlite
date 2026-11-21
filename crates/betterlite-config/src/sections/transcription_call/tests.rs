#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::Settings;
use crate::loader::ConfigLoader;

mod loads_settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("settings load")
        .sections
        .transcription_call
}

fn secret(value: Option<&secrecy::SecretString>) -> &str {
    value.expect("secret").expose_secret()
}

#[test]
fn the_shipped_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/transcription-call-globals.yaml");
    let settings = settings(source, &[]);
    assert_eq!(settings.api_base.as_deref(), Some("https://seed.example"));
    assert_eq!(secret(settings.openai_key.as_ref()), "seed-openai-key");
    assert_eq!(settings.drop_params, Some(false));
}
