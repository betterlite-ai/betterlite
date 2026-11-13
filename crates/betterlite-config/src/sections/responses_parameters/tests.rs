#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::{Raw, Settings};
use crate::loader::{ConfigLoader, parse_view};
use crate::sections::Section as _;

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn source_for(yaml: Option<&str>) -> String {
    yaml.map_or_else(
        || "model_list: []\n".to_owned(),
        |value| format!("model_list: []\nlitellm_settings:\n  drop_params: {value}\n"),
    )
}

fn capturing_log<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("a log handle")))
        .finish();
    let outcome = with_default(subscriber, run);
    let output = fs::read_to_string(log.path()).expect("the captured log");
    (outcome, output)
}

fn loaded(
    source: &str,
    environment: &BTreeMap<String, String>,
) -> (Result<Settings, String>, String) {
    capturing_log(|| {
        ConfigLoader {
            lookup: environment.clone(),
        }
        .load(source)
        .map(|config| config.sections.responses_parameters)
        .map_err(|error| error.to_string())
    })
}

fn section_settings(source: &str, environment: &BTreeMap<String, String>) -> (Settings, String) {
    let (raw, _ignored) = parse_view::<Raw>(source).expect("the section parses");
    capturing_log(|| raw.convert(environment).expect("the section converts"))
}

#[rstest]
#[case::env_unset(&[], None, false, false)]
#[case::env_blank(&[("LITELLM_DROP_PARAMS", "")], None, false, false)]
#[case::env_false(&[("LITELLM_DROP_PARAMS", " false ")], None, false, false)]
#[case::env_zero(&[("LITELLM_DROP_PARAMS", "0")], None, false, false)]
#[case::env_off(&[("LITELLM_DROP_PARAMS", "off")], None, false, false)]
#[case::env_true(&[("LITELLM_DROP_PARAMS", "true")], None, true, false)]
#[case::env_one(&[("LITELLM_DROP_PARAMS", "1")], None, true, false)]
#[case::env_yes(&[("LITELLM_DROP_PARAMS", "yes")], None, true, false)]
#[case::env_invalid(&[("LITELLM_DROP_PARAMS", "wat")], None, true, true)]
#[case::yaml_false_overrides_env(&[("LITELLM_DROP_PARAMS", "true")], Some("false"), false, false)]
#[case::yaml_true_overrides_env(&[("LITELLM_DROP_PARAMS", "false")], Some("true"), true, false)]
#[case::yaml_null_uses_env(&[("LITELLM_DROP_PARAMS", "true")], Some("null"), true, false)]
fn loads_drop_params(
    #[case] environment: &[(&str, &str)],
    #[case] yaml: Option<&str>,
    #[case] expected: bool,
    #[case] warns: bool,
) {
    let variables = variables(environment);
    let (outcome, log) = loaded(&source_for(yaml), &variables);
    let settings = outcome.expect("the config loads");
    assert_eq!(settings.drop_params, expected, "{log}");
    assert_eq!(
        log.contains("LITELLM_DROP_PARAMS") && log.contains("treating it as on"),
        warns,
        "{log}"
    );
}

#[rstest]
#[case::text(
    "wat",
    false,
    "litellm_settings.drop_params='wat' is not a flag value, treating it as off"
)]
#[case::integer(
    "2",
    false,
    "litellm_settings.drop_params=2 is not a flag value, treating it as off"
)]
#[case::mapping(
    "{}",
    false,
    "litellm_settings.drop_params={} is not a flag value, treating it as off"
)]
#[case::list(
    "[]",
    false,
    "litellm_settings.drop_params=[] is not a flag value, treating it as off"
)]
fn loads_drop_params_yaml_flags(
    #[case] value: &str,
    #[case] expected: bool,
    #[case] warning: &str,
) {
    let variables = variables(&[("LITELLM_DROP_PARAMS", "true")]);
    let (settings, log) = section_settings(&source_for(Some(value)), &variables);
    assert_eq!(settings.drop_params, expected);
    assert!(log.contains(warning), "{log}");
}

#[test]
fn loads_drop_params_environment_warning_uses_the_pinned_text() {
    let variables = variables(&[("LITELLM_DROP_PARAMS", "wat")]);
    let (settings, log) = section_settings("model_list: []\n", &variables);
    assert!(settings.drop_params);
    assert!(
        log.contains(
            "LITELLM_DROP_PARAMS='wat' is not a flag value, treating it as on. Set it to true or false"
        ),
        "{log}"
    );
}

#[test]
fn loads_drop_params_accepts_an_absent_key_without_an_unsupported_error() {
    let (outcome, _log) = loaded("model_list: []\n", &variables(&[]));
    assert!(!outcome.expect("an absent key loads").drop_params);
}
