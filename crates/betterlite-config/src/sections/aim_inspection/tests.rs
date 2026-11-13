#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

const AIM_CONFIG: &str =
    "guardrails:\n  - guardrail_name: aim-safety\n    litellm_params:\n      guardrail: aim\n";

fn resolve(
    source: &str,
    lookup: &BTreeMap<String, String>,
) -> Result<super::Settings, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

#[test]
fn aim_inspection_config_reads_the_api_key_from_the_environment() {
    let lookup = BTreeMap::from([("AIM_API_KEY".to_owned(), "env-key".to_owned())]);
    let settings = resolve(AIM_CONFIG, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.guardrail_name.as_deref(), Some("aim-safety"));
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.api_base, super::DEFAULT_API_BASE);
    assert!(!configured.inspect_embeddings);
}

#[test]
fn aim_inspection_config_reads_a_configured_base_and_flag() {
    let source = format!(
        "{AIM_CONFIG}      api_key: literal-key\n      api_base: http://aim.test\n      inspect_embeddings: true\n"
    );
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_key.expose_secret(), "literal-key");
    assert_eq!(configured.api_base, "http://aim.test");
    assert!(configured.inspect_embeddings);
}

#[test]
fn aim_inspection_config_reads_the_base_from_the_environment() {
    let lookup = BTreeMap::from([
        ("AIM_API_KEY".to_owned(), "env-key".to_owned()),
        ("AIM_API_BASE".to_owned(), "http://env-aim.test".to_owned()),
    ]);
    let settings = resolve(AIM_CONFIG, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_base, "http://env-aim.test");
}

#[test]
fn aim_inspection_config_requires_an_api_key() {
    let error = resolve(AIM_CONFIG, &BTreeMap::new()).unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "AIM_API_KEY" && path == "guardrails.0.litellm_params.api_key")
    );
}

#[test]
fn aim_inspection_config_skips_other_guardrail_kinds() {
    let source = AIM_CONFIG.replace("guardrail: aim", "guardrail: akto");
    let lookup = BTreeMap::from([("AIM_API_KEY".to_owned(), "env-key".to_owned())]);
    let settings = resolve(&source, &lookup).unwrap();
    assert!(settings.guardrails.is_empty());
}

#[test]
fn aim_inspection_config_is_optional() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader
        .load("general_settings:\n  master_key: test-master\n")
        .unwrap();
    assert!(config.sections.aim_inspection.guardrails.is_empty());
}

#[test]
fn aim_inspection_config_resolves_an_environment_reference_in_the_key() {
    let source = AIM_CONFIG.replace(
        "guardrail: aim\n",
        "guardrail: aim\n      api_key: os.environ/AIM_KEY\n",
    );
    let lookup = BTreeMap::from([("AIM_KEY".to_owned(), "referenced-key".to_owned())]);
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.guardrails.first().unwrap();
    assert_eq!(configured.api_key.expose_secret(), "referenced-key");
}
