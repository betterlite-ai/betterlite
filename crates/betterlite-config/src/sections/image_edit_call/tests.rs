#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.image_edit_call)
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::unset_defaults(6_000.0_f64, &[])]
#[case::environment_value(7.0_f64, &[("REQUEST_TIMEOUT", "7")])]
#[case::fractional_environment_value(0.5_f64, &[("REQUEST_TIMEOUT", "0.5")])]
#[case::empty_environment_is_unset(6_000.0_f64, &[("REQUEST_TIMEOUT", "")])]
fn loads_settings(#[case] expected: f64, #[case] variables: &[(&str, &str)]) {
    let settings = settings("model_list: []\n", variables).expect("settings load");
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        expected.to_bits()
    );
}

#[test]
fn loads_settings_rejects_an_invalid_timeout() {
    let error = settings("model_list: []\n", &[("REQUEST_TIMEOUT", "soon")]).unwrap_err();
    assert!(error.contains("REQUEST_TIMEOUT"), "{error}");
}

#[test]
fn loads_settings_reads_the_shipped_seed() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/image-edit-call-request-timeout.yaml");
    let lookup = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("seed load");
    assert_eq!(
        lookup
            .sections
            .image_edit_call
            .request_timeout_seconds
            .to_bits(),
        6_000.0_f64.to_bits()
    );
    // the seed also exercises the existing decoder for litellm_settings.request_timeout
    assert_eq!(lookup.litellm_settings.request_timeout, Some(7.0));
}
