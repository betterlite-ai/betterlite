#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::io::Error;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::sections::vigil_runtime::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .vigil_runtime
}

#[rstest]
#[case::every_variable(
    &[("VIGIL_GUARD_URL", "https://vigil.example"), ("VIGIL_GUARD_API_KEY", "sk-vigil")],
    Some("https://vigil.example"),
    Some("sk-vigil")
)]
#[case::absent(&[], None, None)]
#[case::explicit_empty_values(
    &[("VIGIL_GUARD_URL", ""), ("VIGIL_GUARD_API_KEY", "")],
    None,
    None
)]
#[case::url_only(&[("VIGIL_GUARD_URL", "https://vigil.example")], Some("https://vigil.example"), None)]
#[case::key_only(&[("VIGIL_GUARD_API_KEY", "sk-vigil")], None, Some("sk-vigil"))]
fn loads_environment_defaults(
    #[case] variables: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] api_key: Option<&str>,
) {
    let loaded = settings("general_settings: {}\n", variables);
    assert_eq!(loaded.api_base.as_deref(), api_base);
    assert_eq!(
        loaded
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        api_key
    );
}

#[test]
fn loads_the_section_from_yaml_beside_an_injected_environment() {
    let source = "model_list: []\ngeneral_settings: {}\nlitellm_settings: {}\n";
    let loaded = settings(
        source,
        &[
            ("VIGIL_GUARD_URL", "https://vigil.example"),
            ("VIGIL_GUARD_API_KEY", "sk-vigil"),
        ],
    );
    assert_eq!(loaded.api_base.as_deref(), Some("https://vigil.example"));
    assert_eq!(
        loaded
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-vigil")
    );
}

#[test]
fn debug_output_redacts_the_api_key() {
    let loaded = settings(
        "general_settings: {}\n",
        &[
            ("VIGIL_GUARD_URL", "https://vigil.example"),
            ("VIGIL_GUARD_API_KEY", "sk-vigil-secret"),
        ],
    );
    let debug = format!("{loaded:?}");
    assert!(!debug.contains("sk-vigil-secret"));
    assert!(debug.contains("REDACTED"));
}

#[test]
fn replays_the_vigil_runtime_corpus() {
    read_dir("fuzz/corpus/vigil_runtime")
        .unwrap()
        .try_for_each(|entry_result| {
            let entry = entry_result?;
            let bytes = read(entry.path())?;
            if let Ok(source) = String::from_utf8(bytes) {
                let _loaded = ConfigLoader {
                    lookup: BTreeMap::<String, String>::new(),
                }
                .load(&source);
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
