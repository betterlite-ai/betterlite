#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compresr_writeback::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::error::CompresrSettingsError;
use super::{
    CompresrGuardrail, CompresrRuntime, Compressible, DEFAULT_API_BASE, DEFAULT_COMPRESSION_MODEL,
    DEFAULT_COMPRESSION_RATIO, DEFAULT_MAX_BYTES_PER_CALL, DEFAULT_MIN_CHARS_TO_COMPRESS,
};
use crate::compresr_transport::DEFAULT_COMPRESS_TIMEOUT;
use crate::generic_guardrail_contracts::UnreachableFallback;

const HEAD: &str = "guardrails:\n  - guardrail_name: squeeze\n    litellm_params:\n      guardrail: compresr\n      mode: pre_call\n";
const KEY: &str = "      api_key: test-key\n";

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn configured(params: &str, environment: &[(&str, &str)]) -> Configured {
    ConfigLoader {
        lookup: lookup(environment),
    }
    .load(&format!("{HEAD}{params}"))
    .unwrap()
    .sections
    .compresr_writeback
    .guardrails
    .into_iter()
    .next()
    .unwrap()
}

fn build(
    params: &str,
    environment: &[(&str, &str)],
) -> Result<CompresrGuardrail, CompresrSettingsError> {
    let runtime = CompresrRuntime {
        clock: Box::new(|| Duration::ZERO),
        event_hook: None,
    };
    CompresrGuardrail::new(
        &configured(params, environment),
        &lookup(environment),
        runtime,
        reqwest::Client::default(),
    )
}

fn failure(params: &str, environment: &[(&str, &str)]) -> CompresrSettingsError {
    let Err(error) = build(params, environment) else {
        panic!("the constructor must refuse this configuration");
    };
    error
}

fn targets(kinds: &[Compressible]) -> BTreeSet<Compressible> {
    kinds.iter().copied().collect()
}

#[test]
fn litellm_transport_defaults_init() {
    let guardrail = build(KEY, &[]).unwrap();
    let transport = &guardrail.transport.settings;
    assert_eq!(guardrail.guardrail_name, "squeeze");
    assert_eq!(transport.api_base, DEFAULT_API_BASE);
    assert_eq!(transport.model, DEFAULT_COMPRESSION_MODEL);
    assert_eq!(
        transport.target_compression_ratio,
        DEFAULT_COMPRESSION_RATIO
    );
    assert!(transport.coarse && transport.dynamic);
    assert!(transport.dynamic_min_ratio.is_none() && transport.dynamic_max_ratio.is_none());
    assert!(transport.compression_params.is_empty());
    assert_eq!(transport.timeout, DEFAULT_COMPRESS_TIMEOUT);
    assert_eq!(transport.fallback, UnreachableFallback::FailClosed);
}

#[test]
fn litellm_settings_defaults_init() {
    let guardrail = build(KEY, &[]).unwrap();
    let settings = &guardrail.settings;
    assert_eq!(
        settings.min_chars_to_compress,
        DEFAULT_MIN_CHARS_TO_COMPRESS
    );
    assert_eq!(settings.targets, targets(&[Compressible::ToolOutputs]));
    assert_eq!(settings.max_bytes_per_call, DEFAULT_MAX_BYTES_PER_CALL);
    assert!(!settings.allow_bypass_header);
    assert_eq!(DEFAULT_MIN_CHARS_TO_COMPRESS, 500);
    assert_eq!(DEFAULT_MAX_BYTES_PER_CALL, 10 * 1024 * 1024);
}

#[test]
fn explicit_values_init() {
    let params = "      api_key: test-key\n      api_base: https://compresr.test/\n      model: latte_v1\n      unreachable_fallback: fail_open\n      timeout: 2.5\n      target_compression_ratio: 4\n      coarse: false\n      min_chars_to_compress: 10\n      max_bytes_per_call: 0\n      allow_bypass_header: true\n      dynamic: false\n      dynamic_min_ratio: 1.5\n      dynamic_max_ratio: 8\n      compression_params:\n        heuristic_chunking: true\n";
    let guardrail = build(params, &[]).unwrap();
    let transport = &guardrail.transport.settings;
    assert_eq!(transport.api_base, "https://compresr.test");
    assert_eq!(transport.model, "latte_v1");
    assert_eq!(Some(transport.target_compression_ratio), Some(4.0_f64));
    assert!(!transport.coarse && !transport.dynamic);
    assert_eq!(transport.dynamic_min_ratio, Some(1.5_f64));
    assert_eq!(transport.dynamic_max_ratio, Some(8.0_f64));
    assert_eq!(transport.timeout, Duration::from_millis(2500));
    assert_eq!(transport.fallback, UnreachableFallback::FailOpen);
    assert_eq!(
        transport.compression_params.get("heuristic_chunking"),
        Some(&GuardrailValue::Boolean(true))
    );
    let settings = &guardrail.settings;
    assert_eq!(settings.min_chars_to_compress, 10);
    assert_eq!(settings.max_bytes_per_call, 0);
    assert!(settings.allow_bypass_header);
}

#[rstest]
#[case::tool_outputs_off(
    "      compress_tool_outputs: false\n",
    &[]
)]
#[case::system_on("      compress_system: true\n", &[Compressible::ToolOutputs, Compressible::System])]
#[case::history_on("      compress_history: true\n", &[Compressible::ToolOutputs, Compressible::History])]
#[case::last_user_on(
    "      compress_last_user: true\n",
    &[Compressible::ToolOutputs, Compressible::LastUser]
)]
#[case::nested_wins(
    "      compress_system: false\n      optional_params:\n        compress_system: true\n",
    &[Compressible::ToolOutputs, Compressible::System]
)]
fn compression_targets_init(#[case] params: &str, #[case] expected: &[Compressible]) {
    let guardrail = build(&format!("{KEY}{params}"), &[]).unwrap();
    assert_eq!(guardrail.settings.targets, targets(expected));
}

#[test]
fn api_key_from_environment_init() {
    let guardrail = build("", &[("COMPRESR_API_KEY", "env-key")]).unwrap();
    assert_eq!(
        guardrail.transport.settings.model,
        DEFAULT_COMPRESSION_MODEL
    );
}

#[test]
fn empty_configured_api_key_falls_back_to_the_environment_init() {
    build("      api_key: ''\n", &[("COMPRESR_API_KEY", "env-key")]).unwrap();
}

#[rstest]
#[case::absent("", &[])]
#[case::empty_environment("", &[("COMPRESR_API_KEY", "")])]
#[case::empty_configured("      api_key: ''\n", &[])]
fn missing_api_key_init(#[case] params: &str, #[case] environment: &[(&str, &str)]) {
    let error = failure(params, environment);
    assert!(matches!(error, CompresrSettingsError::MissingApiKey));
    assert!(error.to_string().contains("COMPRESR_API_KEY"));
}

#[rstest]
#[case::ftp("ftp://compresr.test")]
#[case::no_scheme("compresr.test")]
#[case::file("file:///etc/passwd")]
#[case::google_metadata("http://metadata.google.internal/computeMetadata")]
#[case::link_local("http://169.254.169.254/latest")]
#[case::decimal_link_local("http://2852039166/")]
#[case::mapped_link_local("http://[::ffff:169.254.169.254]/")]
#[case::azure_wireserver("https://168.63.129.16")]
fn blocked_api_base_init(#[case] api_base: &str) {
    let error = failure(&format!("{KEY}      api_base: '{api_base}'\n"), &[]);
    assert!(matches!(error, CompresrSettingsError::Transport(_)));
}

#[rstest]
#[case::private_range("http://10.0.0.5:8080", "http://10.0.0.5:8080")]
#[case::localhost("http://localhost:9000/", "http://localhost:9000")]
#[case::trailing_slashes("https://compresr.test///", "https://compresr.test")]
fn allowed_api_base_init(#[case] api_base: &str, #[case] expected: &str) {
    let guardrail = build(&format!("{KEY}      api_base: '{api_base}'\n"), &[]).unwrap();
    assert_eq!(guardrail.transport.settings.api_base, expected);
}

#[rstest]
#[case::environment_when_unset("", &[("COMPRESR_API_BASE", "https://env.test/")], "https://env.test")]
#[case::configured_wins("      api_base: https://config.test\n", &[("COMPRESR_API_BASE", "https://env.test")], "https://config.test")]
#[case::empty_environment("", &[("COMPRESR_API_BASE", "")], "https://api.compresr.ai")]
#[case::empty_configured("      api_base: ''\n", &[], "https://api.compresr.ai")]
fn api_base_fallback_init(
    #[case] params: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let guardrail = build(&format!("{KEY}{params}"), environment).unwrap();
    assert_eq!(guardrail.transport.settings.api_base, expected);
}

#[test]
fn negative_byte_cap_init() {
    let error = failure(&format!("{KEY}      max_bytes_per_call: -1\n"), &[]);
    assert!(matches!(error, CompresrSettingsError::ByteCap { .. }));
    assert!(
        error
            .to_string()
            .contains("max_bytes_per_call must be >= 0")
    );
}

#[test]
fn smallest_byte_cap_init() {
    let guardrail = build(&format!("{KEY}      max_bytes_per_call: 1\n"), &[]).unwrap();
    assert_eq!(guardrail.settings.max_bytes_per_call, 1);
}

#[test]
fn negative_minimum_reads_as_no_minimum_init() {
    let guardrail = build(&format!("{KEY}      min_chars_to_compress: -5\n"), &[]).unwrap();
    assert_eq!(guardrail.settings.min_chars_to_compress, 0);
}

#[test]
fn unusable_timeout_init() {
    let error = failure(&format!("{KEY}      timeout: -1\n"), &[]);
    assert!(matches!(error, CompresrSettingsError::Timeout { .. }));
}

#[test]
fn retrieval_by_default_creates_the_recovery_store() {
    let guardrail = build(KEY, &[]).unwrap();
    assert!(guardrail.recovery.is_some());
    assert_eq!(
        *guardrail.no_scope_warning_until.lock().unwrap(),
        Duration::ZERO
    );
}

#[rstest]
#[case::flat("      enable_retrieval: false\n")]
#[case::nested("      optional_params:\n        enable_retrieval: false\n")]
fn disabled_retrieval_has_no_recovery_store(#[case] params: &str) {
    let guardrail = build(&format!("{KEY}{params}"), &[]).unwrap();
    assert!(guardrail.recovery.is_none());
}

#[rstest]
#[case::lowercase("true", true)]
#[case::uppercase("TRUE", true)]
#[case::mixed("True", true)]
#[case::false_value("false", false)]
#[case::empty("", false)]
#[case::yes("yes", false)]
#[case::padded(" true", false)]
fn allowed_bypass_header(#[case] value: &str, #[case] expected: bool) {
    let guardrail = build(&format!("{KEY}      allow_bypass_header: true\n"), &[]).unwrap();
    let data = request(&format!(
        r#"{{"proxy_server_request":{{"headers":{{"x-compresr-bypass":"{value}"}}}}}}"#
    ));
    assert_eq!(guardrail.should_bypass(&data), expected);
}

#[rstest]
#[case::no_request("{}")]
#[case::request_not_an_object(r#"{"proxy_server_request":"text"}"#)]
#[case::no_headers(r#"{"proxy_server_request":{}}"#)]
#[case::headers_not_an_object(r#"{"proxy_server_request":{"headers":"text"}}"#)]
#[case::other_header(r#"{"proxy_server_request":{"headers":{"x-other":"true"}}}"#)]
#[case::header_not_text(r#"{"proxy_server_request":{"headers":{"x-compresr-bypass":true}}}"#)]
fn ignored_bypass_header(#[case] body: &str) {
    let guardrail = build(&format!("{KEY}      allow_bypass_header: true\n"), &[]).unwrap();
    assert!(!guardrail.should_bypass(&request(body)));
}

#[test]
fn disallowed_bypass_header() {
    let guardrail = build(KEY, &[]).unwrap();
    let data = request(r#"{"proxy_server_request":{"headers":{"x-compresr-bypass":"true"}}}"#);
    assert!(!guardrail.should_bypass(&data));
}

fn request(body: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(body).unwrap()
}
