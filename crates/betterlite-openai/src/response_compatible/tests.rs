#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::openai_like_settings::Settings;

mod compatible_provider_environment;

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .openai_like_settings
}

fn config(environment: &[(&str, &str)]) -> super::OpenAiLikeResponsesConfig {
    super::OpenAiLikeResponsesConfig {
        settings: settings(environment),
    }
}

fn header_map(headers: &[(&str, &str)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
