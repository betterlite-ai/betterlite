#![cfg(test)]

use std::collections::BTreeMap;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn required_environment() -> Vec<(&'static str, &'static str)> {
    vec![
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "files"),
    ]
}

mod loads_settings;
