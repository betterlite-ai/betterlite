#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::{
    DEFAULT_AUDIENCE, DEFAULT_ISSUER, DEFAULT_TIMEOUT, DEFAULT_TTL_SECONDS, SettingsError,
};
use super::{SignerOptions, SignerSettings};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn base_options() -> SignerOptions {
    SignerOptions {
        issuer: None,
        audience: None,
        ttl_seconds: None,
        access_token_discovery_uri: None,
        token_introspection_endpoint: None,
        verify_issuer: None,
        verify_audience: None,
        end_user_claim_sources: None,
        add_claims: None,
        set_claims: None,
        remove_claims: None,
        channel_token_audience: None,
        channel_token_ttl: None,
        required_claims: None,
        optional_claims: None,
        debug_headers: false,
        allowed_scopes: None,
        timeout: None,
    }
}

fn resolve(options: &SignerOptions, environment: &BTreeMap<String, String>) -> SignerSettings {
    SignerSettings::resolve(options, environment).expect("settings")
}

#[test]
fn an_option_value_wins_over_the_environment() {
    let mut options = base_options();
    options.issuer = Some("https://option.example".to_owned());
    options.audience = Some("option-audience".to_owned());
    options.ttl_seconds = Some(600);
    let settings = resolve(
        &options,
        &env(&[
            ("MCP_JWT_ISSUER", "https://env.example"),
            ("LITELLM_EXTERNAL_URL", "https://external.example"),
            ("MCP_JWT_AUDIENCE", "env-audience"),
            ("MCP_JWT_TTL_SECONDS", "120"),
        ]),
    );
    assert_eq!(settings.issuer, "https://option.example");
    assert_eq!(settings.audience, "option-audience");
    assert_eq!(settings.ttl_seconds, 600);
}

#[test]
fn an_empty_option_issuer_falls_through_to_the_issuer_environment_variable() {
    let mut options = base_options();
    options.issuer = Some(String::new());
    let settings = resolve(
        &options,
        &env(&[
            ("MCP_JWT_ISSUER", "https://env.example"),
            ("LITELLM_EXTERNAL_URL", "https://external.example"),
        ]),
    );
    assert_eq!(settings.issuer, "https://env.example");
}

#[test]
fn an_empty_issuer_variable_falls_through_to_the_external_url() {
    let settings = resolve(
        &base_options(),
        &env(&[
            ("MCP_JWT_ISSUER", ""),
            ("LITELLM_EXTERNAL_URL", "https://external.example"),
        ]),
    );
    assert_eq!(settings.issuer, "https://external.example");
}

#[test]
fn the_issuer_defaults_to_litellm() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(settings.issuer, DEFAULT_ISSUER);
}

#[test]
fn the_audience_defaults_to_mcp() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(settings.audience, DEFAULT_AUDIENCE);
}

#[test]
fn an_empty_audience_variable_counts_as_unset() {
    let settings = resolve(&base_options(), &env(&[("MCP_JWT_AUDIENCE", "")]));
    assert_eq!(settings.audience, DEFAULT_AUDIENCE);
}

#[test]
fn the_ttl_defaults_to_three_hundred_seconds() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(settings.ttl_seconds, DEFAULT_TTL_SECONDS);
}

#[test]
fn an_empty_ttl_variable_counts_as_unset() {
    let settings = resolve(&base_options(), &env(&[("MCP_JWT_TTL_SECONDS", "")]));
    assert_eq!(settings.ttl_seconds, DEFAULT_TTL_SECONDS);
}

#[test]
fn the_ttl_reads_the_environment_when_the_option_is_absent() {
    let settings = resolve(&base_options(), &env(&[("MCP_JWT_TTL_SECONDS", "120")]));
    assert_eq!(settings.ttl_seconds, 120);
}

#[rstest]
#[case::zero(Some(0), 0)]
#[case::negative(Some(-5), -5)]
#[case::environment_zero(None, 0)]
#[case::environment_negative(None, -1)]
fn a_non_positive_ttl_is_an_error(#[case] option: Option<i64>, #[case] ttl: i64) {
    let mut options = base_options();
    options.ttl_seconds = option;
    let environment = if option.is_some() {
        env(&[])
    } else {
        let text = ttl.to_string();
        env(&[("MCP_JWT_TTL_SECONDS", text.as_str())])
    };
    let error = SignerSettings::resolve(&options, &environment).expect_err("non-positive ttl");
    assert_eq!(error, SettingsError::Ttl { ttl_seconds: ttl });
    assert_eq!(
        error.to_string(),
        format!("ttl_seconds must be > 0, got {ttl}")
    );
}

#[test]
fn a_ttl_that_is_not_a_number_names_the_variable() {
    let environment = env(&[("MCP_JWT_TTL_SECONDS", "soon")]);
    let error = SignerSettings::resolve(&base_options(), &environment).expect_err("bad ttl");
    assert!(error.to_string().contains("MCP_JWT_TTL_SECONDS"));
}

#[test]
fn the_channel_token_ttl_follows_the_resolved_ttl() {
    let mut options = base_options();
    options.ttl_seconds = Some(900);
    let settings = resolve(&options, &env(&[]));
    assert_eq!(settings.channel_token_ttl, 900);
}

#[test]
fn an_explicit_channel_token_ttl_is_kept() {
    let mut options = base_options();
    options.ttl_seconds = Some(900);
    options.channel_token_ttl = Some(60);
    let settings = resolve(&options, &env(&[]));
    assert_eq!(settings.channel_token_ttl, 60);
}

#[test]
fn an_explicit_zero_channel_token_ttl_is_kept() {
    let mut options = base_options();
    options.channel_token_ttl = Some(0);
    let settings = resolve(&options, &env(&[]));
    assert_eq!(settings.channel_token_ttl, 0);
}

#[test]
fn end_user_claim_sources_default_to_the_subject_then_the_proxy_user() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(
        settings.end_user_claim_sources,
        ["token:sub".to_owned(), "litellm:user_id".to_owned()]
    );
}

#[test]
fn an_empty_end_user_claim_source_list_takes_the_defaults() {
    let mut options = base_options();
    options.end_user_claim_sources = Some(Vec::new());
    let settings = resolve(&options, &env(&[]));
    assert_eq!(
        settings.end_user_claim_sources,
        ["token:sub".to_owned(), "litellm:user_id".to_owned()]
    );
}

#[test]
fn a_configured_end_user_claim_source_list_is_kept() {
    let mut options = base_options();
    options.end_user_claim_sources = Some(vec!["token:email".to_owned()]);
    let settings = resolve(&options, &env(&[]));
    assert_eq!(settings.end_user_claim_sources, ["token:email".to_owned()]);
}

#[test]
fn allowed_scopes_stay_none_when_absent() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(settings.allowed_scopes, None);
}

#[test]
fn an_explicit_empty_allowed_scope_list_stays_empty() {
    let mut options = base_options();
    options.allowed_scopes = Some(Vec::new());
    let settings = resolve(&options, &env(&[]));
    assert_eq!(settings.allowed_scopes, Some(Vec::new()));
}

#[test]
fn the_timeout_defaults_to_six_hundred_seconds() {
    let settings = resolve(&base_options(), &env(&[]));
    assert_eq!(settings.timeout, DEFAULT_TIMEOUT);
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(600));
}

#[test]
fn debug_headers_default_to_off_and_are_kept_when_set() {
    let settings = resolve(&base_options(), &env(&[]));
    assert!(!settings.debug_headers);
    let mut options = base_options();
    options.debug_headers = true;
    assert!(resolve(&options, &env(&[])).debug_headers);
}

#[test]
fn the_claim_settings_carry_every_configured_value() {
    let mut options = base_options();
    options.issuer = Some("https://issuer.example".to_owned());
    options.audience = Some("audience".to_owned());
    options.ttl_seconds = Some(45);
    options.allowed_scopes = Some(vec!["mcp:tools/call".to_owned()]);
    options.add_claims = Some(Object::from([(
        "tier".to_owned(),
        Json::Text("gold".to_owned()),
    )]));
    options.set_claims = Some(Object::from([(
        "env".to_owned(),
        Json::Text("prod".to_owned()),
    )]));
    options.remove_claims = Some(vec!["nbf".to_owned()]);
    options.required_claims = Some(vec!["sub".to_owned()]);
    options.optional_claims = Some(vec!["groups".to_owned()]);
    options.end_user_claim_sources = Some(vec!["token:email".to_owned()]);
    let settings = resolve(&options, &env(&[]));
    let claims = settings.claim_settings();
    assert_eq!(claims.issuer, "https://issuer.example");
    assert_eq!(claims.audience, "audience");
    assert_eq!(claims.ttl_seconds, 45);
    assert_eq!(
        claims.allowed_scopes,
        Some(vec!["mcp:tools/call".to_owned()])
    );
    assert_eq!(claims.add_claims["tier"], Json::Text("gold".to_owned()));
    assert_eq!(claims.set_claims["env"], Json::Text("prod".to_owned()));
    assert_eq!(claims.remove_claims, ["nbf".to_owned()]);
    assert_eq!(claims.required_claims, ["sub".to_owned()]);
    assert_eq!(claims.optional_claims, ["groups".to_owned()]);
    assert_eq!(claims.end_user_claim_sources, ["token:email".to_owned()]);
}

#[test]
fn the_verification_settings_are_carried_through() {
    let mut options = base_options();
    options.access_token_discovery_uri = Some("https://idp.example/discovery".to_owned());
    options.token_introspection_endpoint = Some("https://idp.example/introspect".to_owned());
    options.verify_issuer = Some("https://idp.example".to_owned());
    options.verify_audience = Some("api://app".to_owned());
    options.channel_token_audience = Some("channel".to_owned());
    options.timeout = Some(Duration::from_secs(5));
    let settings = resolve(&options, &env(&[]));
    assert_eq!(
        settings.access_token_discovery_uri.as_deref(),
        Some("https://idp.example/discovery")
    );
    assert_eq!(
        settings.token_introspection_endpoint.as_deref(),
        Some("https://idp.example/introspect")
    );
    assert_eq!(
        settings.verify_issuer.as_deref(),
        Some("https://idp.example")
    );
    assert_eq!(settings.verify_audience.as_deref(), Some("api://app"));
    assert_eq!(settings.channel_token_audience.as_deref(), Some("channel"));
    assert_eq!(settings.timeout, Duration::from_secs(5));
}
