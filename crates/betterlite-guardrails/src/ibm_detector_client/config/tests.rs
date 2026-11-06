#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::ibm_guardrails::Configured;
use betterlite_config::tls::Verification;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::IbmDetectorConfig;
use crate::ibm_detector_client::client::IbmDetectorClient;

const DETECTOR_SERVER: &str = "guardrails:
  - guardrail_name: ibm
    litellm_params:
      guardrail: ibm_guardrails
      auth_token: token
      base_url: https://detector.test
      detector_id: jailbreak
";

fn load(source: &str, environment: &[(&str, &str)]) -> Configured {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(source).expect("config");
    config
        .sections
        .ibm_guardrails
        .guardrails
        .into_iter()
        .next()
        .expect("one IBM guardrail")
}

fn config_from(source: &str, environment: &[(&str, &str)]) -> IbmDetectorConfig {
    IbmDetectorConfig::try_from(load(source, environment)).expect("settings")
}

#[test]
fn config_builds_the_detector_server_url() {
    let config = config_from(DETECTOR_SERVER, &[]);
    assert_eq!(config.base_url, "https://detector.test");
    assert_eq!(config.api_url, "https://detector.test/api/v1/text/contents");
}

#[test]
fn config_builds_the_orchestrator_url() {
    let source = DETECTOR_SERVER.replace(
        "      detector_id: jailbreak\n",
        "      detector_id: jailbreak\n      is_detector_server: false\n",
    );
    let config = config_from(&source, &[]);
    assert_eq!(
        config.api_url,
        "https://detector.test/api/v2/text/detection/content"
    );
}

#[test]
fn config_appends_to_a_base_url_with_a_trailing_slash() {
    let source = DETECTOR_SERVER.replace("https://detector.test", "https://detector.test/");
    let config = config_from(&source, &[]);
    assert_eq!(
        config.api_url,
        "https://detector.test//api/v1/text/contents"
    );
}

#[test]
fn config_reads_the_auth_token_from_the_environment() {
    let source = DETECTOR_SERVER.replace("      auth_token: token\n", "");
    let config = config_from(&source, &[("IBM_GUARDRAILS_AUTH_TOKEN", "env-token")]);
    assert_eq!(config.auth_token.expose_secret(), "env-token");
}

#[rstest]
#[case::verification_enabled("true", Verification::Enabled)]
#[case::verification_disabled("false", Verification::Disabled)]
fn config_applies_the_ssl_verification_setting(
    #[case] verify_ssl: &str,
    #[case] expected: Verification,
) {
    let source = DETECTOR_SERVER.replace(
        "      detector_id: jailbreak\n",
        &format!("      detector_id: jailbreak\n      verify_ssl: {verify_ssl}\n"),
    );
    let config = config_from(&source, &[]);
    assert_eq!(config.ssl_verification, expected);
    let _client = IbmDetectorClient::new(config).expect("client");
}

#[test]
fn config_applies_extra_headers_over_the_defaults() {
    let source = DETECTOR_SERVER.replace(
        "      detector_id: jailbreak\n",
        "      detector_id: jailbreak
      optional_params:
        extra_headers:
          Authorization: Custom token
          content-type: text/plain
          X-Tenant: acme
",
    );
    let config = config_from(&source, &[]);
    let client = IbmDetectorClient::new(config).expect("client");
    let headers = client.request_headers(Some("jailbreak")).expect("headers");
    assert_eq!(headers["authorization"].to_str().unwrap(), "Custom token");
    assert_eq!(headers["content-type"].to_str().unwrap(), "text/plain");
    assert_eq!(headers["detector-id"].to_str().unwrap(), "jailbreak");
    assert_eq!(headers["x-tenant"].to_str().unwrap(), "acme");
}
