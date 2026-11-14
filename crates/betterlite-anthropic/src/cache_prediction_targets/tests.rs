#![cfg(test)]

use std::sync::Mutex;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::cache_prediction_targets::headers::supported_prediction_headers;
use crate::cache_prediction_targets::messages_url::MessagesUrl;
use crate::cache_prediction_targets::target::{
    NativePredictionTarget, UnsupportedPredictionTarget, WireRequest, resolve_baseline_target,
    resolve_prediction_target, supported_baseline_recipient,
};
use crate::credentials::AnthropicSettings;

mod count_cache_plan;
mod count_prompt;
mod scope;
mod support;

use support::{KEY, MODEL, Resolution, headers, params, settings};

const NATIVE_KEY: &str = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x"}"#;

fn public() -> AnthropicSettings {
    settings("https://api.anthropic.com", None)
}

fn resolve(json: &str) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    let resolution = Resolution::new();
    resolve_prediction_target(&params(json), &public(), &resolution.context())
}

fn resolve_with(
    json: &str,
    settings: &AnthropicSettings,
) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    let resolution = Resolution::new();
    resolve_prediction_target(&params(json), settings, &resolution.context())
}

fn baseline(json: &str) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    let resolution = Resolution::new();
    resolve_baseline_target(&params(json), &public(), &resolution.context())
}

#[test]
fn deployment_config_extra_option_accepts_the_deployment_options() {
    let json = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"https://api.anthropic.com","custom_llm_provider":"anthropic","rpm":10,"tpm":20,"timeout":30,"stream_timeout":5,"max_retries":2,"num_retries":2,"max_parallel_requests":4,"input_cost_per_token":0.1,"output_cost_per_token":0.2,"cache_read_input_token_cost":0.01,"cache_creation_input_token_cost":0.02,"cache_creation_input_token_cost_above_1hr":0.03}"#;
    assert_eq!(resolve(json).unwrap().model, "claude-x");
}

#[rstest]
#[case::api_version(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_version":"2024-01-01"}"#
)]
#[case::organization(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":"org"}"#
)]
#[case::pass_through(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","use_in_pass_through":true}"#
)]
#[case::max_budget(r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","max_budget":10}"#)]
#[case::chat_completions_false(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","use_chat_completions_api":false}"#
)]
fn deployment_config_extra_option_is_rejected(#[case] json: &str) {
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::DeploymentConfiguration
    );
}

#[rstest]
#[case::pass_through_false(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","use_in_pass_through":false}"#
)]
#[case::proxy_false(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","use_litellm_proxy":false}"#
)]
#[case::oauth_flag_false(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","use_xai_oauth":false}"#
)]
fn deployment_config_extra_option_allows_a_false_default(#[case] json: &str) {
    assert_eq!(resolve(json).expect("resolved").model, MODEL);
}

#[test]
fn deployment_config_extra_option_reasons_are_the_litellm_strings() {
    assert_eq!(
        UnsupportedPredictionTarget::DeploymentConfiguration.reason(),
        "unsupported_deployment_configuration"
    );
    assert_eq!(
        UnsupportedPredictionTarget::ProviderEndpoint.reason(),
        "unsupported_provider_endpoint"
    );
    assert_eq!(
        UnsupportedPredictionTarget::Provider.reason(),
        "unsupported_provider"
    );
    assert_eq!(
        UnsupportedPredictionTarget::ProviderCredentials.reason(),
        "unsupported_provider_credentials"
    );
}

#[test]
fn deployment_config_extra_option_ignores_null_values() {
    let json = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":null}"#;
    assert_eq!(resolve(json).expect("resolved").model, MODEL);
}

#[rstest]
#[case::configured("https://configured.test")]
#[case::lookalike("https://anthropic.example.test")]
#[case::trailing_slash("https://api.anthropic.com/")]
#[case::plain_http("http://api.anthropic.com")]
fn endpoint_non_public_rejects_a_configured_base(#[case] api_base: &str) {
    let json = format!(
        r#"{{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"{api_base}"}}"#
    );
    assert_eq!(
        resolve(&json).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderEndpoint
    );
}

#[rstest]
#[case::base("https://api.anthropic.com")]
#[case::messages("https://api.anthropic.com/v1/messages")]
fn endpoint_non_public_accepts_the_public_endpoints(#[case] api_base: &str) {
    let json = format!(
        r#"{{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"{api_base}"}}"#
    );
    assert_eq!(resolve(&json).unwrap().api_base, api_base);
}

#[test]
fn endpoint_non_public_reads_the_settings_base() {
    let configured = settings("https://configured.test", None);
    assert_eq!(
        resolve_with(NATIVE_KEY, &configured).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderEndpoint
    );
    let public = settings("https://api.anthropic.com/v1/messages", None);
    assert_eq!(
        resolve_with(NATIVE_KEY, &public).unwrap().api_base,
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn endpoint_non_public_treats_an_empty_base_as_unset() {
    let json = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":""}"#;
    assert_eq!(resolve(json).unwrap().api_base, "https://api.anthropic.com");
}

#[test]
fn endpoint_non_public_runs_before_the_provider_check() {
    let json = r#"{"model":"openai/gpt-4o","api_key":"sk-ant-api03-x","api_base":"https://configured.test"}"#;
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderEndpoint
    );
}

#[rstest]
#[case::openai(r#"{"model":"openai/gpt-4o","api_key":"sk-ant-api03-x"}"#)]
#[case::azure(r#"{"model":"azure/gpt-4o","api_key":"sk-ant-api03-x"}"#)]
#[case::bare(r#"{"model":"claude-x","api_key":"sk-ant-api03-x"}"#)]
#[case::declared(
    r#"{"model":"claude-x","custom_llm_provider":"openai","api_key":"sk-ant-api03-x"}"#
)]
fn provider_not_anthropic_rejects_another_provider(#[case] json: &str) {
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::Provider
    );
}

#[rstest]
#[case::prefixed(r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x"}"#)]
#[case::declared(
    r#"{"model":"claude-x","custom_llm_provider":"anthropic","api_key":"sk-ant-api03-x"}"#
)]
fn provider_not_anthropic_strips_the_prefix_from_the_model(#[case] json: &str) {
    assert_eq!(resolve(json).unwrap().model, MODEL);
}

#[test]
fn provider_not_anthropic_rejects_a_text_model_family() {
    let json = r#"{"model":"anthropic/claude-2","api_key":"sk-ant-api03-x"}"#;
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::Provider
    );
}

#[test]
fn provider_not_anthropic_rejects_a_non_string_provider() {
    let json = r#"{"model":"claude-x","custom_llm_provider":5,"api_key":"sk-ant-api03-x"}"#;
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::Provider
    );
}

#[test]
fn credentials_missing_or_oauth_rejects_a_missing_key() {
    assert_eq!(
        resolve(r#"{"model":"anthropic/claude-x"}"#).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderCredentials
    );
}

#[rstest]
#[case::empty(r#"{"model":"anthropic/claude-x","api_key":""}"#)]
#[case::oauth(r#"{"model":"anthropic/claude-x","api_key":"sk-ant-oat01-abc"}"#)]
#[case::oauth_bearer(r#"{"model":"anthropic/claude-x","api_key":"Bearer sk-ant-oat01-abc"}"#)]
#[case::not_a_string(r#"{"model":"anthropic/claude-x","api_key":5}"#)]
fn credentials_missing_or_oauth_rejects_the_key(#[case] json: &str) {
    assert_eq!(
        resolve(json).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderCredentials
    );
}

#[rstest]
#[case::explicit("sk-ant-api03-key")]
#[case::bearer_wrapped("Bearer sk-ant-api03-key")]
fn credentials_missing_or_oauth_accepts_a_supported_key(#[case] presented: &str) {
    let json = format!(r#"{{"model":"anthropic/claude-x","api_key":"{presented}"}}"#);
    assert_eq!(resolve(&json).unwrap().api_key.expose_secret(), presented);
}

#[test]
fn credentials_missing_or_oauth_falls_back_to_the_settings_key() {
    let settings = settings("https://api.anthropic.com", Some(KEY));
    let json = r#"{"model":"anthropic/claude-x"}"#;
    assert_eq!(
        resolve_with(json, &settings)
            .unwrap()
            .api_key
            .expose_secret(),
        KEY
    );
}

#[test]
fn credentials_missing_or_oauth_prefers_the_explicit_key() {
    let settings = settings("https://api.anthropic.com", Some(KEY));
    let json = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-explicit"}"#;
    assert_eq!(
        resolve_with(json, &settings)
            .unwrap()
            .api_key
            .expose_secret(),
        "sk-ant-api03-explicit"
    );
}

#[test]
fn credentials_missing_or_oauth_treats_an_empty_explicit_key_as_unset() {
    let settings = settings("https://api.anthropic.com", Some(KEY));
    let json = r#"{"model":"anthropic/claude-x","api_key":""}"#;
    assert_eq!(
        resolve_with(json, &settings)
            .unwrap()
            .api_key
            .expose_secret(),
        KEY
    );
}

#[test]
fn credentials_missing_or_oauth_rejects_an_empty_settings_key() {
    let settings = settings("https://api.anthropic.com", Some(""));
    let json = r#"{"model":"anthropic/claude-x"}"#;
    assert_eq!(
        resolve_with(json, &settings).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderCredentials
    );
}

#[test]
fn resolve_baseline_accepts_a_configured_endpoint() {
    let json = r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"https://configured.test"}"#;
    let target = baseline(json).unwrap();
    assert_eq!(target.api_base, "https://configured.test");
    assert_eq!(target.model, MODEL);
}

#[test]
fn resolve_baseline_accepts_the_public_endpoint() {
    assert_eq!(
        baseline(NATIVE_KEY).unwrap().api_base,
        "https://api.anthropic.com"
    );
}

#[test]
fn resolve_baseline_rejects_unsupported_options() {
    let json =
        r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_version":"2024-01-01"}"#;
    assert_eq!(
        baseline(json).expect_err("rejected"),
        UnsupportedPredictionTarget::DeploymentConfiguration
    );
}

#[test]
fn resolve_baseline_rejects_a_non_anthropic_provider() {
    let json = r#"{"model":"openai/gpt-4o","api_key":"sk-ant-api03-x","api_base":"https://configured.test"}"#;
    assert_eq!(
        baseline(json).expect_err("rejected"),
        UnsupportedPredictionTarget::Provider
    );
}

#[test]
fn resolve_baseline_rejects_missing_credentials() {
    let json = r#"{"model":"anthropic/claude-x","api_base":"https://configured.test"}"#;
    assert_eq!(
        baseline(json).expect_err("rejected"),
        UnsupportedPredictionTarget::ProviderCredentials
    );
}

#[rstest]
#[case::lower("anthropic-version")]
#[case::upper("Anthropic-Version")]
#[case::mixed("AnThRoPiC-VeRsIoN")]
fn headers_accept_the_default_version_in_any_case(#[case] name: &str) {
    assert!(supported_prediction_headers(&headers(&[(
        name,
        "2023-06-01"
    )])));
}

#[test]
fn headers_accept_an_empty_header_map() {
    assert!(supported_prediction_headers(&headers(&[])));
}

#[test]
fn headers_accept_unrelated_headers() {
    assert!(supported_prediction_headers(&headers(&[
        ("x-other", "y"),
        ("anthropic-version", "2023-06-01"),
    ])));
}

#[rstest]
#[case::lower("anthropic-beta", "")]
#[case::upper("Anthropic-Beta", "prompt-caching-2024-07-31")]
#[case::mixed("AnThRoPiC-BeTa", "x")]
fn headers_reject_anthropic_beta_in_any_case(#[case] name: &str, #[case] value: &str) {
    assert!(!supported_prediction_headers(&headers(&[(name, value)])));
}

#[test]
fn headers_reject_another_anthropic_version() {
    assert!(!supported_prediction_headers(&headers(&[(
        "anthropic-version",
        "2024-01-01"
    )])));
}

#[test]
fn headers_reject_an_empty_anthropic_version() {
    assert!(!supported_prediction_headers(&headers(&[(
        "anthropic-version",
        ""
    )])));
}

fn native_target() -> NativePredictionTarget {
    resolve(NATIVE_KEY).unwrap()
}

#[test]
fn baseline_recipient_accepts_the_native_request() {
    let target = native_target();
    let headers = headers(&[("x-api-key", "sk-ant-api03-x")]);
    let url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_accepts_a_case_insensitive_key_and_normalized_url() {
    let target = native_target();
    let headers = headers(&[("X-Api-Key", "sk-ant-api03-x")]);
    let url = Url::parse("https://API.anthropic.com:443/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_rejects_another_key() {
    let target = native_target();
    let headers = headers(&[("x-api-key", "sk-ant-api03-other")]);
    let url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(!supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_rejects_a_missing_key() {
    let target = native_target();
    let headers = headers(&[]);
    let url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(!supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_rejects_another_url() {
    let target = native_target();
    let headers = headers(&[("x-api-key", "sk-ant-api03-x")]);
    let url = Url::parse("https://configured.test/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(!supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_rejects_a_query() {
    let target = native_target();
    let headers = headers(&[("x-api-key", "sk-ant-api03-x")]);
    let url = Url::parse("https://api.anthropic.com/v1/messages?x=1").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    assert!(!supported_baseline_recipient(&target, &wire, &public()));
}

#[test]
fn baseline_recipient_uses_the_injected_url_builder() {
    let target = native_target();
    let headers = headers(&[("x-api-key", "sk-ant-api03-x")]);
    let url = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
    let wire = WireRequest {
        headers: &headers,
        url: &url,
    };
    let builder = Recorder::default();
    assert!(!supported_baseline_recipient(&target, &wire, &builder));
    assert_eq!(
        builder.base.lock().unwrap().as_deref(),
        Some("https://api.anthropic.com")
    );
}

#[derive(Default)]
struct Recorder {
    base: Mutex<Option<String>>,
}

impl MessagesUrl for Recorder {
    fn messages_url(&self, api_base: Option<&str>) -> String {
        *self.base.lock().unwrap() = api_base.map(str::to_owned);
        String::from("https://elsewhere.test/v1/messages")
    }
}
