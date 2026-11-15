#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::batch_managed_records::Settings;

const VARIABLE: &str = "PROXY_BATCH_POLLING_ENABLED";

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(config.sections.batch_managed_records)
}

#[rstest]
#[case::unset_defaults_on(&[], true)]
#[case::explicit_true(&[("PROXY_BATCH_POLLING_ENABLED", "true")], true)]
#[case::upper_case_true(&[("PROXY_BATCH_POLLING_ENABLED", "TRUE")], true)]
#[case::explicit_false(&[("PROXY_BATCH_POLLING_ENABLED", "false")], false)]
#[case::numeric_one(&[("PROXY_BATCH_POLLING_ENABLED", "1")], true)]
#[case::yes_is_on(&[("PROXY_BATCH_POLLING_ENABLED", "yes")], true)]
#[case::off_is_false(&[("PROXY_BATCH_POLLING_ENABLED", "off")], false)]
#[case::empty_is_unset(&[("PROXY_BATCH_POLLING_ENABLED", "")], true)]
fn loads_settings(#[case] environment: &[(&str, &str)], #[case] enabled: bool) {
    let settings = load("{}\n", environment).unwrap();
    assert_eq!(settings.proxy_batch_polling_enabled, enabled);
}

#[test]
fn an_invalid_value_names_the_variable() {
    let error = load("{}\n", &[(VARIABLE, "maybe")]).unwrap_err();
    assert!(error.to_string().contains(VARIABLE), "{error}");
    assert_eq!(error.category(), "environment");
}

// LiteLLM reads the flag once at import, so a YAML environment_variables entry never turns polling off.
#[test]
fn the_environment_variables_key_is_supported() {
    let yaml = format!("environment_variables:\n  {VARIABLE}: \"false\"\n");
    let settings = load(&yaml, &[]).unwrap();
    assert!(settings.proxy_batch_polling_enabled);
}
