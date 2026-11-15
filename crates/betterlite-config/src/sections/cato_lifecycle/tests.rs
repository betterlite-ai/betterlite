#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, parse_view};
use crate::sections::Section as _;
use crate::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};

const CATO_CONFIG: &str = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n      mode: pre_call\n      default_on: true\n      timeout: 12.5\n";

fn resolve(
    source: &str,
    lookup: &BTreeMap<String, String>,
) -> Result<Vec<super::Configured>, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

#[test]
fn cato_lifecycle_config_reads_mode_default_on_and_timeout() {
    let settings = resolve(CATO_CONFIG, &BTreeMap::new()).unwrap();
    let configured = settings.first().unwrap();
    assert_eq!(configured.guardrail_name, "cato");
    assert_eq!(
        configured.mode,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(configured.default_on);
    assert_eq!(configured.timeout, Some(12.5_f64));
}

#[test]
fn cato_lifecycle_config_leaves_absent_keys_at_their_defaults() {
    let source = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n";
    let settings = resolve(source, &BTreeMap::new()).unwrap();
    let configured = settings.first().unwrap();
    assert!(configured.mode.is_none());
    assert!(!configured.default_on);
    assert!(configured.timeout.is_none());
}

#[test]
fn cato_lifecycle_config_skips_other_guardrail_kinds() {
    let source = CATO_CONFIG.replace("guardrail: cato_networks", "guardrail: aim");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert!(settings.is_empty());
}

#[test]
fn cato_lifecycle_config_lowercases_a_string_mode() {
    let source = CATO_CONFIG.replace("mode: pre_call", "mode: Pre_Call");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert_eq!(
        settings.first().unwrap().mode,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
}

#[test]
fn cato_lifecycle_config_reads_a_list_mode() {
    let source = CATO_CONFIG.replace("mode: pre_call", "mode: [Pre_Call, During_Call]");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert_eq!(
        settings.first().unwrap().mode,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "during_call".to_owned()
        ]))
    );
}

#[test]
fn cato_lifecycle_config_reads_a_tagged_mode() {
    let source = CATO_CONFIG.replace(
        "mode: pre_call",
        "mode:\n        tags:\n          Team-A: [Pre_Call]\n        default: Post_Call",
    );
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    let mode = settings.first().unwrap().mode.clone();
    assert_eq!(
        mode,
        Some(LitellmParamsMode::Tagged(Mode {
            tags: BTreeMap::from([(
                "Team-A".to_owned(),
                ModeTarget::Many(vec!["Pre_Call".to_owned()])
            )]),
            default: Some(ModeTarget::One("Post_Call".to_owned())),
        }))
    );
}

#[test]
fn cato_lifecycle_config_reads_an_integer_timeout_as_a_float() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: 30");
    let settings = resolve(source.as_str(), &BTreeMap::new()).unwrap();
    assert_eq!(settings.first().unwrap().timeout, Some(30.0_f64));
}

#[test]
fn cato_lifecycle_config_resolves_an_environment_timeout() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: os.environ/CATO_TIMEOUT");
    let lookup = BTreeMap::from([("CATO_TIMEOUT".to_owned(), "2.5".to_owned())]);
    let settings = resolve(&source, &lookup).unwrap();
    assert_eq!(settings.first().unwrap().timeout, Some(2.5_f64));
}

#[test]
fn cato_lifecycle_config_reads_an_empty_environment_timeout_as_unset() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: os.environ/CATO_TIMEOUT");
    let lookup = BTreeMap::from([("CATO_TIMEOUT".to_owned(), String::new())]);
    let settings = resolve(&source, &lookup).unwrap();
    assert!(settings.first().unwrap().timeout.is_none());
}

#[test]
fn cato_lifecycle_config_reads_an_empty_timeout_as_unset() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: ''");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert!(settings.first().unwrap().timeout.is_none());
}

#[test]
fn cato_lifecycle_config_names_a_missing_timeout_environment_reference() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: os.environ/CATO_TIMEOUT");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
        if variable == "CATO_TIMEOUT" && path == "guardrails.0.litellm_params.timeout"
    ));
}

#[test]
fn cato_lifecycle_config_rejects_a_timeout_that_is_not_a_number() {
    let source = CATO_CONFIG.replace("timeout: 12.5", "timeout: soon");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidFloat { path, .. } if path == "guardrails.0.litellm_params.timeout"
    ));
}
