#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
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
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
    assert_eq!(configured.api_base, super::DEFAULT_API_BASE);
    assert!(!configured.inspect_embeddings);
}

#[test]
fn aim_inspection_config_reads_every_configured_value() {
    let lookup = BTreeMap::from([("AIM_API_KEY".to_owned(), "env-key".to_owned())]);
    let source = format!(
        "{AIM_CONFIG}      api_key: literal-key\n      api_base: http://aim.test\n      inspect_embeddings: true\n"
    );
    let configured = resolve(&source, &lookup).unwrap().guardrails.remove(0);
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "literal-key"
    );
    assert_eq!(configured.api_base, "http://aim.test");
    assert!(configured.inspect_embeddings);
    let environment = BTreeMap::from([
        ("AIM_API_KEY".to_owned(), "env-key".to_owned()),
        ("AIM_API_BASE".to_owned(), "http://env-aim.test".to_owned()),
    ]);
    let from_env = resolve(AIM_CONFIG, &environment)
        .unwrap()
        .guardrails
        .remove(0);
    assert_eq!(from_env.api_base, "http://env-aim.test");
}

#[test]
fn aim_inspection_config_allows_a_missing_api_key_and_skips_other_kinds() {
    let settings = resolve(AIM_CONFIG, &BTreeMap::new()).unwrap();
    assert!(settings.guardrails.first().unwrap().api_key.is_none());
    let other = AIM_CONFIG.replace("guardrail: aim", "guardrail: akto");
    assert!(
        resolve(&other, &BTreeMap::new())
            .unwrap()
            .guardrails
            .is_empty()
    );
}

#[test]
fn aim_inspection_config_treats_empty_values_as_unset() {
    let source = format!("{AIM_CONFIG}      api_key: \"\"\n      api_base: \"\"\n");
    let lookup = BTreeMap::from([
        ("AIM_API_KEY".to_owned(), "env-key".to_owned()),
        ("AIM_API_BASE".to_owned(), String::new()),
    ]);
    let configured = resolve(&source, &lookup).unwrap().guardrails.remove(0);
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
    assert_eq!(configured.api_base, super::DEFAULT_API_BASE);
    let blank = BTreeMap::from([("AIM_API_KEY".to_owned(), String::new())]);
    let configured = resolve(&source, &blank).unwrap().guardrails.remove(0);
    assert!(configured.api_key.is_none());
}

#[test]
fn aim_inspection_config_falls_back_when_an_environment_reference_is_empty() {
    let source = format!(
        "{AIM_CONFIG}      api_key: os.environ/AIM_KEY\n      api_base: os.environ/AIM_BASE\n"
    );
    let lookup = BTreeMap::from([
        ("AIM_KEY".to_owned(), String::new()),
        ("AIM_BASE".to_owned(), String::new()),
        ("AIM_API_KEY".to_owned(), "env-key".to_owned()),
        ("AIM_API_BASE".to_owned(), "http://env-aim.test".to_owned()),
    ]);
    let configured = resolve(&source, &lookup).unwrap().guardrails.remove(0);
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
    assert_eq!(configured.api_base, "http://env-aim.test");
}

#[test]
fn aim_inspection_config_is_optional_and_resolves_environment_references() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader
        .load("general_settings:\n  master_key: test-master\n")
        .unwrap();
    assert!(config.sections.aim_inspection.guardrails.is_empty());
    let source = AIM_CONFIG.replace(
        "guardrail: aim\n",
        "guardrail: aim\n      api_key: os.environ/AIM_KEY\n",
    );
    let lookup = BTreeMap::from([("AIM_KEY".to_owned(), "referenced-key".to_owned())]);
    let configured = resolve(&source, &lookup).unwrap().guardrails.remove(0);
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "referenced-key"
    );
}

#[test]
fn aim_inspection_config_skips_a_non_aim_entry_before_resolving_its_params() {
    let source = "guardrails:\n  - guardrail_name: other\n    litellm_params:\n      guardrail: custom\n      inspect_embeddings: os.environ/FLAG\n";
    let lookup = BTreeMap::from([("FLAG".to_owned(), "perhaps".to_owned())]);
    assert!(resolve(source, &lookup).unwrap().guardrails.is_empty());
}

#[rstest]
#[case::true_text("true", true)]
#[case::off_text("off", false)]
#[case::empty_text("", false)]
fn aim_inspection_config_reads_the_embedding_flag_from_the_environment(
    #[case] text: &str,
    #[case] expected: bool,
) {
    let source = format!("{AIM_CONFIG}      inspect_embeddings: os.environ/AIM_INSPECT\n");
    let lookup = BTreeMap::from([("AIM_INSPECT".to_owned(), text.to_owned())]);
    let configured = resolve(&source, &lookup).unwrap().guardrails.remove(0);
    assert_eq!(
        configured.inspect_embeddings, expected,
        "AIM_INSPECT: {text}"
    );
}

#[test]
fn aim_inspection_config_counts_an_unset_embedding_flag_as_false() {
    let source = format!("{AIM_CONFIG}      inspect_embeddings: os.environ/AIM_INSPECT\n");
    let unset = resolve(&source, &BTreeMap::new())
        .unwrap()
        .guardrails
        .remove(0);
    assert!(!unset.inspect_embeddings);
}

#[test]
fn aim_inspection_config_rejects_a_bad_embedding_flag_in_the_environment() {
    let source = format!("{AIM_CONFIG}      inspect_embeddings: os.environ/AIM_INSPECT\n");
    let lookup = BTreeMap::from([("AIM_INSPECT".to_owned(), "perhaps".to_owned())]);
    assert!(matches!(
        resolve(&source, &lookup),
        Err(ConfigError::Environment { .. })
    ));
}

#[test]
fn aim_inspection_config_treats_a_null_embedding_flag_as_the_default() {
    let source = format!("{AIM_CONFIG}      inspect_embeddings: null\n");
    let configured = resolve(&source, &BTreeMap::new())
        .unwrap()
        .guardrails
        .remove(0);
    assert!(!configured.inspect_embeddings);
}

#[rstest]
#[case::quoted_bool("\"true\"")]
#[case::integer("1")]
#[case::float("1.5")]
fn aim_inspection_config_rejects_an_embedding_flag_of_another_type(#[case] value: &str) {
    let source = format!("{AIM_CONFIG}      inspect_embeddings: {value}\n");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::InvalidScalar { path, .. }
                if path == "guardrails.0.litellm_params.inspect_embeddings"
        ),
        "inspect_embeddings: {value}"
    );
}
