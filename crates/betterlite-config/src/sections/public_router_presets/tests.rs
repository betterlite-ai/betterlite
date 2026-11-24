#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const DEFAULT_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/litellm/proxy/public_endpoints/autorouter_presets.json";
const FLAG_ENV: &str = "LITELLM_LOCAL_AUTOROUTER_PRESETS";
const URL_ENV: &str = "LITELLM_AUTOROUTER_PRESETS_URL";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .public_router_presets)
}

#[rstest]
#[case::flag_omitted("", &[], false, DEFAULT_URL)]
#[case::flag_empty("", &[(FLAG_ENV, "")], false, DEFAULT_URL)]
#[case::flag_false("", &[(FLAG_ENV, "false")], false, DEFAULT_URL)]
#[case::flag_one("", &[(FLAG_ENV, "1")], false, DEFAULT_URL)]
#[case::flag_padded_true("", &[(FLAG_ENV, " true ")], false, DEFAULT_URL)]
#[case::flag_lowercase_true("", &[(FLAG_ENV, "true")], true, DEFAULT_URL)]
#[case::flag_uppercase_true("", &[(FLAG_ENV, "TRUE")], true, DEFAULT_URL)]
#[case::flag_title_case_true("", &[(FLAG_ENV, "True")], true, DEFAULT_URL)]
#[case::url_from_environment("", &[(URL_ENV, "http://presets.test/catalog.json")], false, "http://presets.test/catalog.json")]
#[case::url_empty_environment("", &[(URL_ENV, "")], false, "")]
#[case::url_padded_environment("", &[(URL_ENV, " http://presets.test/catalog.json ")], false, " http://presets.test/catalog.json ")]
#[case::url_settings_over_environment(
    "litellm_settings:\n  autorouter_presets_url: http://yaml.test/catalog.json\n",
    &[(URL_ENV, "http://presets.test/catalog.json")],
    false,
    "http://yaml.test/catalog.json"
)]
#[case::url_empty_settings_over_environment(
    "litellm_settings:\n  autorouter_presets_url: ''\n",
    &[(URL_ENV, "http://presets.test/catalog.json")],
    false,
    ""
)]
#[case::url_null_settings_keep_environment(
    "litellm_settings:\n  autorouter_presets_url:\n",
    &[(URL_ENV, "http://presets.test/catalog.json")],
    false,
    "http://presets.test/catalog.json"
)]
#[case::url_settings_environment_reference(
    "litellm_settings:\n  autorouter_presets_url: os.environ/PRESETS_URL\n",
    &[("PRESETS_URL", "http://mirror.test/catalog.json")],
    false,
    "http://mirror.test/catalog.json"
)]
fn loads_router_preset_settings(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] local_only: bool,
    #[case] url: &str,
) {
    let settings = load(source, environment).unwrap();
    assert_eq!(settings.local_only, local_only);
    assert_eq!(settings.url, url);
}

#[test]
fn rejects_a_url_of_the_wrong_type() {
    let error = load("litellm_settings:\n  autorouter_presets_url: [a]\n", &[]).unwrap_err();
    assert!(
        error.to_string().contains("autorouter_presets_url"),
        "{error}"
    );
}

#[test]
fn rejects_an_environment_reference_that_names_no_variable() {
    let source = "litellm_settings:\n  autorouter_presets_url: os.environ/PRESETS_URL\n";
    let error = load(source, &[]).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("PRESETS_URL"), "{message}");
    assert!(
        message.contains("litellm_settings.autorouter_presets_url"),
        "{message}"
    );
}

#[test]
fn default_settings_fetch_remotely_from_the_litellm_catalog() {
    let settings = super::Settings::default();
    assert!(!settings.local_only);
    assert_eq!(settings.url, DEFAULT_URL);
}

#[test]
fn the_shipped_valid_seed_sets_the_remote_url() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/public-router-presets-valid.yaml");
    let settings = load(source, &[]).unwrap();
    assert!(!settings.local_only);
    assert_eq!(settings.url, "https://presets.test/catalog.json");
}

#[test]
fn the_shipped_empty_url_seed_keeps_the_url_empty() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/public-router-presets-empty-url.yaml");
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.url, "");
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source =
        include_str!("../../../fuzz/corpus/config_yaml/public-router-presets-invalid.yaml");
    let error = load(source, &[]).unwrap_err();
    assert!(
        error.to_string().contains("autorouter_presets_url"),
        "{error}"
    );
}
