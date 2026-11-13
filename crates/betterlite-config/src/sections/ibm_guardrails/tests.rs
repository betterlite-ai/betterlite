#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{Configured, DetectorConfigModel};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use crate::sections::guardrail_identity::item::GuardrailValue;
use crate::tls::Verification;

const MINIMAL: &str = "guardrails:
  - guardrail_name: ibm-jailbreak
    litellm_params:
      guardrail: ibm_guardrails
";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Vec<Configured>, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.ibm_guardrails.guardrails)
}

fn with_params(lines: &str) -> String {
    format!("{MINIMAL}{lines}")
}

#[test]
fn ibm_guardrails_section_names_the_guardrail_for_the_ui() {
    assert_eq!(
        DetectorConfigModel::ui_friendly_name(),
        "IBM Guardrails Detector"
    );
}

#[test]
fn ibm_guardrails_section_applies_the_constructor_defaults() {
    let guardrails = load(MINIMAL, &[]).unwrap();
    let entry = guardrails.first().unwrap();
    assert_eq!(entry.guardrail_name, "ibm-jailbreak");
    assert!(entry.auth_token.is_none());
    assert!(entry.base_url.is_none());
    assert!(entry.detector_id.is_none());
    assert!(entry.is_detector_server);
    assert_eq!(entry.ssl_verification, Verification::Enabled);
    assert!(entry.block_on_detection);
    assert!(entry.detector_params.is_empty());
    assert!(entry.extra_headers.is_empty());
    assert_eq!(entry.score_threshold, None);
    assert_eq!(entry.timeout, None);
}

#[test]
fn ibm_guardrails_section_skips_other_guardrail_kinds() {
    let source = MINIMAL.replace("ibm_guardrails", "aim");
    assert!(load(&source, &[]).unwrap().is_empty());
}

#[test]
fn ibm_guardrails_section_loads_every_key() {
    let source = with_params(
        "      auth_token: token-1
      base_url: https://detector.test
      detector_id: jailbreak-detector
      is_detector_server: false
      verify_ssl: false
      timeout: 5
      optional_params:
        detector_params: {threshold: 3, labels: [a, b]}
        extra_headers: {X-Tenant: acme}
        score_threshold: 0.8
        block_on_detection: false
",
    );
    let guardrails = load(&source, &[]).unwrap();
    let entry = guardrails.first().unwrap();
    assert_eq!(
        entry.auth_token.as_ref().unwrap().expose_secret(),
        "token-1"
    );
    assert_eq!(entry.base_url.as_deref(), Some("https://detector.test"));
    assert_eq!(entry.detector_id.as_deref(), Some("jailbreak-detector"));
    assert_eq!(entry.timeout, Some(Duration::from_secs(5)));
    assert!(!entry.is_detector_server);
    assert_eq!(entry.ssl_verification, Verification::Disabled);
    assert!(!entry.block_on_detection);
    assert_eq!(entry.score_threshold, Some(0.8_f64));
    assert_eq!(entry.extra_headers["X-Tenant"], "acme");
    assert_eq!(
        entry.detector_params["labels"],
        GuardrailValue::Array(vec![
            GuardrailValue::Text("a".to_owned()),
            GuardrailValue::Text("b".to_owned()),
        ])
    );
    assert_eq!(
        entry.detector_params["threshold"],
        GuardrailValue::SignedInteger(3)
    );
}

#[rstest]
#[case::absent_parameter_uses_the_environment("", "env-token", Some("env-token"))]
#[case::parameter_wins_over_the_environment(
    "      auth_token: param-token\n",
    "env-token",
    Some("param-token")
)]
#[case::explicit_empty_parameter_is_used_as_given("      auth_token: ''\n", "env-token", Some(""))]
#[case::empty_environment_variable_counts_as_unset("", "", None)]
#[case::reference_reads_the_named_variable(
    "      auth_token: os.environ/OTHER_TOKEN\n",
    "env-token",
    Some("other-token")
)]
fn ibm_guardrails_section_resolves_the_auth_token(
    #[case] line: &str,
    #[case] environment_token: &str,
    #[case] expected: Option<&str>,
) {
    let source = with_params(line);
    let environment = [
        ("IBM_GUARDRAILS_AUTH_TOKEN", environment_token),
        ("OTHER_TOKEN", "other-token"),
    ];
    let guardrails = load(&source, &environment).unwrap();
    let token = guardrails.first().unwrap().auth_token.as_ref();
    assert_eq!(token.map(secrecy::ExposeSecret::expose_secret), expected);
}

#[test]
fn ibm_guardrails_section_names_a_missing_reference() {
    let source = with_params("      auth_token: os.environ/NOT_SET\n");
    let error = load(&source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, .. } if variable == "NOT_SET"
    ));
}

#[test]
fn ibm_guardrails_section_resolves_base_url_and_detector_id_references() {
    let source = with_params(
        "      base_url: os.environ/IBM_URL
      detector_id: os.environ/IBM_DETECTOR
",
    );
    let environment = [
        ("IBM_URL", "https://from-env.test"),
        ("IBM_DETECTOR", "from-env-detector"),
    ];
    let guardrails = load(&source, &environment).unwrap();
    let entry = guardrails.first().unwrap();
    assert_eq!(entry.base_url.as_deref(), Some("https://from-env.test"));
    assert_eq!(entry.detector_id.as_deref(), Some("from-env-detector"));
}

#[test]
fn ibm_guardrails_section_keeps_explicit_empty_strings() {
    let source = with_params("      base_url: ''\n      detector_id: ''\n");
    let guardrails = load(&source, &[]).unwrap();
    let entry = guardrails.first().unwrap();
    assert_eq!(entry.base_url.as_deref(), Some(""));
    assert_eq!(entry.detector_id.as_deref(), Some(""));
}

#[rstest]
#[case::detector_server("      is_detector_server: null\n", true)]
#[case::explicit_false("      is_detector_server: false\n", false)]
fn ibm_guardrails_section_reads_null_as_the_default(#[case] line: &str, #[case] expected: bool) {
    let guardrails = load(&with_params(line), &[]).unwrap();
    assert_eq!(guardrails.first().unwrap().is_detector_server, expected);
}

#[test]
fn ibm_guardrails_section_reads_null_optional_values_as_defaults() {
    let source = with_params(
        "      verify_ssl: null
      timeout: null
      optional_params:
        detector_params: null
        extra_headers: null
        score_threshold: null
        block_on_detection: null
",
    );
    let guardrails = load(&source, &[]).unwrap();
    let entry = guardrails.first().unwrap();
    assert_eq!(entry.ssl_verification, Verification::Enabled);
    assert!(entry.block_on_detection);
    assert!(entry.detector_params.is_empty());
    assert!(entry.extra_headers.is_empty());
    assert_eq!(entry.score_threshold, None);
    assert_eq!(entry.timeout, None);
}

#[rstest]
#[case::seconds("5", Some(Duration::from_secs(5)))]
#[case::fraction("0.5", Some(Duration::from_millis(500)))]
#[case::numeric_string("'2.5'", Some(Duration::from_millis(2500)))]
#[case::null("null", None)]
#[case::empty_string("''", None)]
fn ibm_guardrails_section_reads_the_configured_timeout(
    #[case] line: &str,
    #[case] expected: Option<Duration>,
) {
    let source = with_params(&format!("      timeout: {line}\n"));
    let guardrails = load(&source, &[]).unwrap();
    assert_eq!(guardrails.first().unwrap().timeout, expected);
}

#[test]
fn ibm_guardrails_section_reads_a_timeout_reference() {
    let source = with_params("      timeout: os.environ/IBM_TIMEOUT\n");
    let guardrails = load(&source, &[("IBM_TIMEOUT", "2.5")]).unwrap();
    assert_eq!(
        guardrails.first().unwrap().timeout,
        Some(Duration::from_millis(2500))
    );
}

#[test]
fn ibm_guardrails_section_rejects_a_negative_timeout() {
    let source = with_params("      timeout: -1\n");
    load(&source, &[]).unwrap_err();
}

#[test]
fn ibm_guardrails_section_keeps_an_explicit_zero_threshold() {
    let source = with_params("      optional_params:\n        score_threshold: 0\n");
    let guardrails = load(&source, &[]).unwrap();
    assert_eq!(guardrails.first().unwrap().score_threshold, Some(0.0_f64));
}

#[test]
fn ibm_guardrails_section_loads_each_entry_in_order() {
    let source = "guardrails:
  - guardrail_name: first
    litellm_params: {guardrail: ibm_guardrails, detector_id: one}
  - guardrail_name: other
    litellm_params: {guardrail: aim}
  - guardrail_name: second
    litellm_params: {guardrail: ibm_guardrails, detector_id: two}
";
    let guardrails = load(source, &[]).unwrap();
    let names: Vec<_> = guardrails
        .iter()
        .map(|entry| entry.guardrail_name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn ibm_guardrails_section_reads_the_config_model() {
    let model: DetectorConfigModel = serde_json::from_str(
        r#"{"base_url": "https://detector.test", "verify_ssl": false, "is_detector_server": null,
            "optional_params": {"score_threshold": 0.5, "block_on_detection": false}}"#,
    )
    .unwrap();
    assert_eq!(model.base_url.as_deref(), Some("https://detector.test"));
    assert_eq!(model.is_detector_server, None);
    assert_eq!(model.verify_ssl, Some(false));
    let optional = model.optional_params.unwrap();
    assert_eq!(optional.score_threshold, Some(0.5_f64));
    assert_eq!(optional.block_on_detection, Some(false));
}
