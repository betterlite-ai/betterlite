use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::singulr::DEFAULT_API_BASE;
use crate::singulr::settings::{SettingsError, SingulrSettings};

use super::support;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolve(pairs: &[(&str, &str)]) -> Result<SingulrSettings, SettingsError> {
    SingulrSettings::resolve(&support::args(), &env(pairs))
}

#[test]
fn the_api_base_defaults_to_the_local_gateway() {
    assert_eq!(resolve(&[]).unwrap().api_base, DEFAULT_API_BASE);
}

#[test]
fn the_api_base_reads_its_environment_variable() {
    let settings = resolve(&[("SINGULR_API_BASE", "https://guard.example")]).unwrap();
    assert_eq!(settings.api_base, "https://guard.example");
}

#[test]
fn the_api_base_argument_wins_over_the_environment() {
    let mut args = support::args();
    args.singulr_api_base = Some("https://argument.example".to_owned());
    let settings = SingulrSettings::resolve(
        &args,
        &env(&[("SINGULR_API_BASE", "https://guard.example")]),
    )
    .unwrap();
    assert_eq!(settings.api_base, "https://argument.example");
}

#[test]
fn an_empty_api_base_environment_value_counts_as_unset() {
    let settings = resolve(&[("SINGULR_API_BASE", "")]).unwrap();
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
}

#[test]
fn an_empty_api_base_argument_is_used_as_given() {
    let mut args = support::args();
    args.singulr_api_base = Some(String::new());
    let settings = SingulrSettings::resolve(
        &args,
        &env(&[("SINGULR_API_BASE", "https://guard.example")]),
    )
    .unwrap();
    assert_eq!(settings.api_base, "");
}

#[test]
fn the_api_base_is_trimmed_and_loses_its_trailing_slashes() {
    let settings = resolve(&[("SINGULR_API_BASE", "  https://guard.example///  ")]).unwrap();
    assert_eq!(settings.api_base, "https://guard.example");
}

#[test]
fn plain_http_to_a_non_local_host_is_refused_and_keeps_the_token_out() {
    let error = resolve(&[("SINGULR_API_BASE", "http://guard.example")]).unwrap_err();
    assert!(
        matches!(&error, SettingsError::PlainHttp { api_base } if api_base == "http://guard.example")
    );
    let message = error.to_string();
    assert!(message.contains("http://guard.example"));
    assert!(message.contains("HTTPS"));
    assert!(!message.contains(support::API_KEY));
}

#[rstest]
#[case::localhost("http://localhost:8003")]
#[case::loopback("http://127.0.0.1:8003")]
#[case::uppercase_host("http://LOCALHOST:8003")]
fn plain_http_to_a_local_host_is_allowed(#[case] base: &str) {
    let settings = resolve(&[("SINGULR_API_BASE", base)]).unwrap();
    assert_eq!(settings.api_base, base);
}

#[test]
fn https_to_a_non_local_host_is_allowed() {
    let settings = resolve(&[("SINGULR_API_BASE", "https://guard.example/")]).unwrap();
    assert_eq!(settings.api_base, "https://guard.example");
}

#[test]
fn a_base_that_is_not_a_url_is_not_refused_at_start() {
    let settings = resolve(&[("SINGULR_API_BASE", "guard.example:8003")]).unwrap();
    assert_eq!(settings.api_base, "guard.example:8003");
}

#[test]
fn the_api_key_argument_wins_over_the_environment() {
    let mut args = support::args();
    args.singulr_api_key = Some(secrecy::SecretString::from("argument-key"));
    let settings =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_API_KEY", "env-key")])).unwrap();
    assert_eq!(
        settings.api_key.as_ref().unwrap().expose_secret(),
        "argument-key"
    );
}

#[test]
fn the_api_key_reads_its_environment_variable() {
    let mut args = support::args();
    args.singulr_api_key = None;
    let settings =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_API_KEY", "env-key")])).unwrap();
    assert_eq!(
        settings.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
}

#[test]
fn an_empty_api_key_environment_value_counts_as_unset() {
    let mut args = support::args();
    args.singulr_api_key = None;
    let settings = SingulrSettings::resolve(&args, &env(&[("SINGULR_API_KEY", "")])).unwrap();
    assert!(settings.api_key.is_none());
}

#[test]
fn the_ids_read_their_environment_variables() {
    let mut args = support::args();
    args.singulr_application_id = None;
    args.singulr_guardrail_id = None;
    let settings = SingulrSettings::resolve(
        &args,
        &env(&[
            ("SINGULR_ENFORCEMENT_ENTITY_ID", "entity-1"),
            ("SINGULR_GUARDRAIL_ID", "guardrail-9"),
        ]),
    )
    .unwrap();
    assert_eq!(settings.application_id.as_deref(), Some("entity-1"));
    assert_eq!(settings.guardrail_id.as_deref(), Some("guardrail-9"));
}

#[test]
fn the_ids_default_to_none() {
    let mut args = support::args();
    args.singulr_application_id = None;
    args.singulr_guardrail_id = None;
    let settings = SingulrSettings::resolve(&args, &env(&[])).unwrap();
    assert!(settings.application_id.is_none());
    assert!(settings.guardrail_id.is_none());
}

#[test]
fn block_on_error_defaults_to_true() {
    let mut args = support::args();
    args.block_on_error = None;
    let settings = SingulrSettings::resolve(&args, &env(&[])).unwrap();
    assert!(settings.block_on_error);
}

#[rstest]
#[case::true_spelling("true", true)]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::on("on", true)]
#[case::upper_case("TRUE", true)]
#[case::false_spelling("false", false)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::off("off", false)]
fn block_on_error_reads_the_environment(#[case] value: &str, #[case] expected: bool) {
    let mut args = support::args();
    args.block_on_error = None;
    let settings =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_BLOCK_ON_ERROR", value)])).unwrap();
    assert_eq!(settings.block_on_error, expected);
}

#[test]
fn an_empty_block_on_error_environment_value_counts_as_unset() {
    let mut args = support::args();
    args.block_on_error = None;
    let settings =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_BLOCK_ON_ERROR", "")])).unwrap();
    assert!(settings.block_on_error);
}

#[test]
fn an_unparsable_block_on_error_is_an_error() {
    let mut args = support::args();
    args.block_on_error = None;
    let error =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_BLOCK_ON_ERROR", "maybe")])).unwrap_err();
    assert!(matches!(error, SettingsError::BlockOnError { .. }));
}

#[test]
fn the_block_on_error_argument_wins_over_the_environment() {
    let mut args = support::args();
    args.block_on_error = Some(false);
    let settings =
        SingulrSettings::resolve(&args, &env(&[("SINGULR_BLOCK_ON_ERROR", "true")])).unwrap();
    assert!(!settings.block_on_error);
}

#[test]
fn the_timeout_defaults_to_thirty_seconds() {
    let mut args = support::args();
    args.timeout = None;
    let settings = SingulrSettings::resolve(&args, &env(&[])).unwrap();
    assert_eq!(settings.timeout, Duration::from_secs(30));
}

#[test]
fn the_timeout_is_the_argument() {
    let mut args = support::args();
    args.timeout = Some(2.5);
    let settings = SingulrSettings::resolve(&args, &env(&[])).unwrap();
    assert_eq!(settings.timeout, Duration::try_from_secs_f64(2.5).unwrap());
}

#[test]
fn a_negative_timeout_is_an_error() {
    let mut args = support::args();
    args.timeout = Some(-1.0);
    assert!(matches!(
        SingulrSettings::resolve(&args, &env(&[])).unwrap_err(),
        SettingsError::Timeout { .. }
    ));
}
