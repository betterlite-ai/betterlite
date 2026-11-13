use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::VerifySetting;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cato::error::CatoError;
use crate::cato::{
    CallHook, CatoGuardrail, CatoParams, DEFAULT_TIMEOUT, SUPPORTED_EVENT_HOOKS, WsSsl,
};

const CONFIG: &str = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n      api_key: config-key\n      api_base: https://config.test/\n";

fn params() -> CatoParams {
    CatoParams {
        guardrail_name: Some("cato".to_owned()),
        api_key: Some(SecretString::from("test-key")),
        api_base: Some("https://cato.test".to_owned()),
        inspect_embeddings: None,
        ssl_verify: None,
        timeout: None,
    }
}

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn build(params: CatoParams, lookup: &BTreeMap<String, String>) -> CatoGuardrail {
    CatoGuardrail::new(params, lookup).expect("guardrail")
}

fn configured(source: &str, lookup: &BTreeMap<String, String>) -> CatoGuardrail {
    let config = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load(source)
    .expect("config");
    let entry = config
        .sections
        .cato_projections
        .first()
        .expect("cato guardrail");
    let params = CatoParams {
        guardrail_name: Some(entry.guardrail_name.clone()),
        api_key: entry.api_key.clone(),
        api_base: entry.api_base.clone(),
        inspect_embeddings: Some(entry.inspect_embeddings),
        ssl_verify: entry.ssl_verify.clone(),
        timeout: None,
    };
    build(params, lookup)
}

#[test]
fn cato_guardrail_constructor_requires_an_api_key() {
    let mut params = params();
    params.api_key = None;
    let error = CatoGuardrail::new(params, &BTreeMap::new()).expect_err("missing key");
    assert!(matches!(error, CatoError::MissingApiKey));
}

#[test]
fn cato_guardrail_constructor_reads_the_api_key_from_the_environment() {
    let mut params = params();
    params.api_key = None;
    let lookup = environment(&[("CATO_API_KEY", "env-key")]);
    let guardrail = build(params, &lookup);
    assert_eq!(guardrail.api_key().expose_secret(), "env-key");
}

#[test]
fn cato_guardrail_constructor_prefers_the_explicit_api_key() {
    let lookup = environment(&[("CATO_API_KEY", "env-key")]);
    let guardrail = build(params(), &lookup);
    assert_eq!(guardrail.api_key().expose_secret(), "test-key");
}

#[test]
fn cato_guardrail_constructor_treats_an_empty_environment_key_as_unset() {
    let mut params = params();
    params.api_key = None;
    let lookup = environment(&[("CATO_API_KEY", "")]);
    let error = CatoGuardrail::new(params, &lookup).expect_err("missing key");
    assert!(matches!(error, CatoError::MissingApiKey));
}

#[test]
fn cato_guardrail_constructor_keeps_the_api_key_out_of_debug_output() {
    let mut params = params();
    params.api_key = Some(SecretString::from("super-secret-key"));
    let guardrail = build(params, &BTreeMap::new());
    let rendered = format!("{guardrail:?}");
    assert!(!rendered.contains("super-secret-key"));
    assert!(rendered.contains("REDACTED"));
}

#[test]
fn cato_guardrail_constructor_defaults_the_api_base() {
    let mut params = params();
    params.api_base = None;
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.api_base, "https://api.aisec.catonetworks.com");
    assert_eq!(guardrail.ws_api_base, "wss://api.aisec.catonetworks.com");
}

#[test]
fn cato_guardrail_constructor_reads_the_api_base_from_the_environment() {
    let mut params = params();
    params.api_base = None;
    let lookup = environment(&[("CATO_API_BASE", "https://env.test/")]);
    let guardrail = build(params, &lookup);
    assert_eq!(guardrail.api_base, "https://env.test");
    assert_eq!(guardrail.ws_api_base, "wss://env.test");
}

#[test]
fn cato_guardrail_constructor_treats_an_empty_environment_base_as_unset() {
    let mut params = params();
    params.api_base = None;
    let lookup = environment(&[("CATO_API_BASE", "")]);
    let guardrail = build(params, &lookup);
    assert_eq!(guardrail.api_base, "https://api.aisec.catonetworks.com");
}

#[rstest]
#[case::https_with_slash("https://cato.test/", "https://cato.test", "wss://cato.test")]
#[case::https_with_many_slashes("https://cato.test///", "https://cato.test", "wss://cato.test")]
#[case::http_becomes_ws(
    "http://cato.test:8080",
    "http://cato.test:8080",
    "ws://cato.test:8080"
)]
#[case::ws_stays_ws("ws://cato.test", "ws://cato.test", "ws://cato.test")]
#[case::wss_with_slash("wss://cato.test/", "wss://cato.test", "wss://cato.test")]
#[case::no_scheme("cato.test", "cato.test", "cato.test")]
fn cato_guardrail_constructor_normalizes_the_api_base_and_derives_websocket_urls(
    #[case] api_base: &str,
    #[case] expected_base: &str,
    #[case] expected_ws: &str,
) {
    let mut params = params();
    params.api_base = Some(api_base.to_owned());
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.api_base, expected_base);
    assert_eq!(guardrail.ws_api_base, expected_ws);
}

#[rstest]
#[case::absent(None, false)]
#[case::set_false(Some(false), false)]
#[case::set_true(Some(true), true)]
fn cato_guardrail_constructor_reads_inspect_embeddings(
    #[case] inspect_embeddings: Option<bool>,
    #[case] expected: bool,
) {
    let mut params = params();
    params.inspect_embeddings = inspect_embeddings;
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.inspect_embeddings, expected);
}

#[test]
fn cato_guardrail_constructor_supports_the_pre_call_during_call_and_post_call_hooks() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        ["pre_call", "during_call", "post_call"]
    );
}

#[test]
fn cato_guardrail_call_hooks_use_the_header_values_litellm_sends() {
    assert_eq!(CallHook::PreCall.header_value(), "pre_call");
    assert_eq!(CallHook::Moderation.header_value(), "moderation");
    assert_eq!(CallHook::Output.header_value(), "output");
}

#[test]
fn cato_guardrail_constructor_uses_the_default_timeout() {
    let guardrail = build(params(), &BTreeMap::new());
    assert_eq!(guardrail.timeout, DEFAULT_TIMEOUT);
}

#[test]
fn cato_guardrail_constructor_reads_a_configured_timeout() {
    let mut params = params();
    params.timeout = Some(2.5);
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.timeout, Duration::from_millis(2500));
}

#[test]
fn cato_guardrail_constructor_rejects_a_negative_timeout() {
    let mut params = params();
    params.timeout = Some(-1.0);
    let error = CatoGuardrail::new(params, &BTreeMap::new()).expect_err("timeout");
    assert!(matches!(error, CatoError::TimeoutSetting { .. }));
}

#[test]
fn cato_guardrail_ws_ssl_is_absent_without_an_ssl_verify_setting() {
    let guardrail = build(params(), &BTreeMap::new());
    assert_eq!(guardrail.ws_ssl, None);
}

#[test]
fn cato_guardrail_ws_ssl_is_absent_for_a_websocket_that_is_not_wss() {
    let mut params = params();
    params.api_base = Some("http://cato.test".to_owned());
    params.ssl_verify = Some(VerifySetting::Bool(false));
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.ws_api_base, "ws://cato.test");
    assert_eq!(guardrail.ws_ssl, None);
}

#[test]
fn cato_guardrail_ws_ssl_disables_verification_for_false() {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Bool(false));
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.ws_ssl, Some(WsSsl::Disabled));
}

#[test]
fn cato_guardrail_ws_ssl_uses_the_system_roots_for_true() {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Bool(true));
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.ws_ssl, Some(WsSsl::SystemRoots));
}

#[test]
fn cato_guardrail_ws_ssl_uses_a_configured_ca_file() {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Path("/etc/ca.pem".to_owned()));
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(
        guardrail.ws_ssl,
        Some(WsSsl::CaFile("/etc/ca.pem".to_owned()))
    );
}

#[rstest]
#[case::false_string("false", Some(WsSsl::Disabled))]
#[case::true_string("true", Some(WsSsl::SystemRoots))]
#[case::uppercase("FALSE", Some(WsSsl::Disabled))]
fn cato_guardrail_ws_ssl_reads_a_boolean_string(
    #[case] ssl_verify: &str,
    #[case] expected: Option<WsSsl>,
) {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Path(ssl_verify.to_owned()));
    let guardrail = build(params, &BTreeMap::new());
    assert_eq!(guardrail.ws_ssl, expected);
}

#[test]
fn cato_guardrail_ws_ssl_uses_ssl_cert_file_for_true() {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Bool(true));
    let lookup = environment(&[("SSL_CERT_FILE", "/etc/root.pem")]);
    let guardrail = build(params, &lookup);
    assert_eq!(
        guardrail.ws_ssl,
        Some(WsSsl::CaFile("/etc/root.pem".to_owned()))
    );
}

#[test]
fn cato_guardrail_ws_ssl_ignores_ssl_cert_file_when_verification_is_off() {
    let mut params = params();
    params.ssl_verify = Some(VerifySetting::Bool(false));
    let lookup = environment(&[("SSL_CERT_FILE", "/etc/root.pem")]);
    let guardrail = build(params, &lookup);
    assert_eq!(guardrail.ws_ssl, Some(WsSsl::Disabled));
}

#[test]
fn cato_guardrail_config_loads_the_api_key_and_base() {
    let guardrail = configured(CONFIG, &BTreeMap::new());
    assert_eq!(guardrail.api_key().expose_secret(), "config-key");
    assert_eq!(guardrail.api_base, "https://config.test");
    assert_eq!(guardrail.ws_api_base, "wss://config.test");
    assert!(!guardrail.inspect_embeddings);
    assert_eq!(guardrail.guardrail_name.as_deref(), Some("cato"));
}

#[test]
fn cato_guardrail_config_falls_back_to_the_environment() {
    let source = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n";
    let lookup = environment(&[
        ("CATO_API_KEY", "env-key"),
        ("CATO_API_BASE", "https://env.test"),
    ]);
    let guardrail = configured(source, &lookup);
    assert_eq!(guardrail.api_key().expose_secret(), "env-key");
    assert_eq!(guardrail.api_base, "https://env.test");
}

#[test]
fn cato_guardrail_config_defaults_the_api_base() {
    let source = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n      api_key: config-key\n";
    let guardrail = configured(source, &BTreeMap::new());
    assert_eq!(guardrail.api_base, "https://api.aisec.catonetworks.com");
}

#[test]
fn cato_guardrail_config_reads_inspect_embeddings() {
    let source = CONFIG.replace(
        "guardrail: cato_networks",
        "guardrail: cato_networks\n      inspect_embeddings: true",
    );
    let guardrail = configured(&source, &BTreeMap::new());
    assert!(guardrail.inspect_embeddings);
}

#[test]
fn cato_guardrail_config_reads_ssl_verify() {
    let source = CONFIG.replace(
        "guardrail: cato_networks",
        "guardrail: cato_networks\n      ssl_verify: false",
    );
    let guardrail = configured(&source, &BTreeMap::new());
    assert_eq!(guardrail.ws_ssl, Some(WsSsl::Disabled));
}

#[test]
fn cato_guardrail_config_rejects_a_missing_api_key() {
    let source = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("config");
    let entry = config
        .sections
        .cato_projections
        .first()
        .expect("cato guardrail");
    let params = CatoParams {
        guardrail_name: Some(entry.guardrail_name.clone()),
        api_key: entry.api_key.clone(),
        api_base: entry.api_base.clone(),
        inspect_embeddings: Some(entry.inspect_embeddings),
        ssl_verify: entry.ssl_verify.clone(),
        timeout: None,
    };
    let error = CatoGuardrail::new(params, &BTreeMap::new()).expect_err("missing key");
    assert!(matches!(error, CatoError::MissingApiKey));
}
