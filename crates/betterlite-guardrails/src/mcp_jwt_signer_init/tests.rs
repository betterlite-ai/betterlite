#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use reqwest::Client;
use rstest::rstest;

use super::{
    CallbackRegistry, GuardrailInitError, JwtSignerSlot, SIGNER_TYPE_NAME, WIRE_NAME,
    initialize_mcp_jwt_signer,
};
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::signer_settings::SignerOptions;

struct RecordingRegistry {
    env: BTreeMap<String, String>,
    client: Client,
    callbacks: Vec<Arc<JwtSigner>>,
}

impl RecordingRegistry {
    fn new() -> Self {
        Self {
            env: BTreeMap::new(),
            client: Client::default(),
            callbacks: Vec::new(),
        }
    }
}

impl CallbackRegistry for RecordingRegistry {
    fn add_litellm_callback(&mut self, signer: Arc<JwtSigner>) {
        self.callbacks.push(signer);
    }

    fn environment(&self) -> &dyn EnvironmentLookup {
        &self.env
    }

    fn http_client(&self) -> Client {
        self.client.clone()
    }
}

fn with_params(extra: &str) -> String {
    format!(r#"{{"guardrail": "mcp_jwt_signer", "mode": "pre_mcp_call"{extra}}}"#)
}

fn decode(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).expect("decodable litellm params")
}

fn initialize(json: &str) -> Result<Arc<JwtSigner>, GuardrailInitError> {
    let mut registry = RecordingRegistry::new();
    initialize_mcp_jwt_signer(
        &decode(json),
        Some("jwt"),
        &mut registry,
        &JwtSignerSlot::new(),
    )
}

fn signer(json: &str) -> Arc<JwtSigner> {
    initialize(json).expect("the signer builds")
}

fn error_text(result: Result<Arc<JwtSigner>, GuardrailInitError>) -> String {
    result.err().expect("the initializer refused").to_string()
}

fn generated_signer() -> Arc<JwtSigner> {
    Arc::new(
        JwtSigner::new(
            &SignerOptions::default(),
            &BTreeMap::new(),
            Client::default(),
        )
        .expect("generated signer"),
    )
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn a_missing_guardrail_name_is_refused(#[case] name: Option<&str>) {
    let params = decode(&with_params(""));
    let mut registry = RecordingRegistry::new();
    let slot = JwtSignerSlot::new();
    let refused = initialize_mcp_jwt_signer(&params, name, &mut registry, &slot);
    assert_eq!(
        error_text(refused),
        "MCPJWTSigner guardrail requires a guardrail_name"
    );
    assert!(registry.callbacks.is_empty());
    assert!(slot.get().is_none());
}

#[rstest]
#[case::single(r#""pre_call""#, "pre_call")]
#[case::list(r#"["pre_call", "post_call"]"#, r#"["pre_call","post_call"]"#)]
#[case::tagged(
    r#"{"tags": {"default": "pre_call"}}"#,
    r#"{"tags":{"default":"pre_call"},"default":null}"#
)]
fn a_mode_other_than_pre_mcp_call_is_refused(#[case] mode: &str, #[case] shown: &str) {
    let json = format!(r#"{{"guardrail": "mcp_jwt_signer", "mode": {mode}}}"#);
    assert_eq!(
        error_text(initialize(&json)),
        format!(
            "MCPJWTSigner guardrail 'jwt' has mode='{shown}' but must use mode='pre_mcp_call'. JWT injection only fires for MCP tool calls."
        )
    );
}

#[test]
fn the_name_is_checked_before_the_mode() {
    let params = decode(r#"{"guardrail": "mcp_jwt_signer", "mode": "pre_call"}"#);
    let mut registry = RecordingRegistry::new();
    let refused = initialize_mcp_jwt_signer(&params, None, &mut registry, &JwtSignerSlot::new());
    assert_eq!(
        error_text(refused),
        "MCPJWTSigner guardrail requires a guardrail_name"
    );
}

#[test]
fn string_options_come_from_the_top_level() {
    let signer = signer(&with_params(
        r#",
        "issuer": "top-issuer",
        "audience": "top-audience",
        "access_token_discovery_uri": "https://idp.example/.well-known",
        "token_introspection_endpoint": "https://idp.example/introspect",
        "verify_issuer": "verified-issuer",
        "verify_audience": "verified-audience",
        "channel_token_audience": "channel""#,
    ));
    let settings = &signer.settings;
    assert_eq!(settings.issuer, "top-issuer");
    assert_eq!(settings.audience, "top-audience");
    assert_eq!(
        settings.access_token_discovery_uri.as_deref(),
        Some("https://idp.example/.well-known")
    );
    assert_eq!(
        settings.token_introspection_endpoint.as_deref(),
        Some("https://idp.example/introspect")
    );
    assert_eq!(settings.verify_issuer.as_deref(), Some("verified-issuer"));
    assert_eq!(
        settings.verify_audience.as_deref(),
        Some("verified-audience")
    );
    assert_eq!(settings.channel_token_audience.as_deref(), Some("channel"));
}

#[test]
fn list_options_come_from_the_top_level() {
    let signer = signer(&with_params(
        r#",
        "end_user_claim_sources": ["token:sub"],
        "remove_claims": ["gone"],
        "required_claims": ["required"],
        "optional_claims": ["optional"],
        "allowed_scopes": ["scope"],
        "debug_headers": true"#,
    ));
    let settings = &signer.settings;
    assert_eq!(
        settings.end_user_claim_sources,
        vec!["token:sub".to_owned()]
    );
    assert_eq!(settings.remove_claims, vec!["gone".to_owned()]);
    assert_eq!(settings.required_claims, vec!["required".to_owned()]);
    assert_eq!(settings.optional_claims, vec!["optional".to_owned()]);
    assert_eq!(settings.allowed_scopes, Some(vec!["scope".to_owned()]));
    assert!(settings.debug_headers);
}

#[test]
fn number_and_claim_options_come_from_the_top_level() {
    let signer = signer(&with_params(
        r#",
        "ttl_seconds": 600,
        "channel_token_ttl": 60,
        "add_claims": {"add": 1},
        "set_claims": {"set": true},
        "timeout": 1.5"#,
    ));
    let settings = &signer.settings;
    assert_eq!(settings.ttl_seconds, 600);
    assert_eq!(settings.channel_token_ttl, 60);
    assert_eq!(
        serde_json::to_string(&settings.add_claims).expect("claims json"),
        r#"{"add":1}"#
    );
    assert_eq!(
        serde_json::to_string(&settings.set_claims).expect("claims json"),
        r#"{"set":true}"#
    );
    assert_eq!(settings.timeout, Duration::from_millis(1500));
}

#[test]
fn optional_params_win_over_the_top_level() {
    let signer = signer(&with_params(
        r#",
        "issuer": "top-issuer",
        "ttl_seconds": 600,
        "debug_headers": true,
        "optional_params": {
            "issuer": "nested-issuer",
            "ttl_seconds": 60,
            "debug_headers": false
        }"#,
    ));
    assert_eq!(signer.settings.issuer, "nested-issuer");
    assert_eq!(signer.settings.ttl_seconds, 60);
    assert!(!signer.settings.debug_headers);
}

#[test]
fn list_and_claim_options_from_optional_params_win() {
    let signer = signer(&with_params(
        r#",
        "end_user_claim_sources": ["top:sub"],
        "add_claims": {"top": true},
        "allowed_scopes": ["top"],
        "optional_params": {
            "end_user_claim_sources": ["token:sub", "litellm:user_id"],
            "add_claims": {"nested": 2},
            "allowed_scopes": []
        }"#,
    ));
    assert_eq!(
        signer.settings.end_user_claim_sources,
        vec!["token:sub".to_owned(), "litellm:user_id".to_owned()]
    );
    assert_eq!(
        serde_json::to_string(&signer.settings.add_claims).expect("claims json"),
        r#"{"nested":2}"#
    );
    assert_eq!(signer.settings.allowed_scopes, Some(Vec::new()));
}

#[test]
fn null_in_optional_params_falls_to_the_top_level() {
    let signer = signer(&with_params(
        r#",
        "issuer": "top-issuer",
        "ttl_seconds": 600,
        "optional_params": {"issuer": null, "ttl_seconds": null}"#,
    ));
    assert_eq!(signer.settings.issuer, "top-issuer");
    assert_eq!(signer.settings.ttl_seconds, 600);
}

#[test]
fn null_in_both_places_leaves_the_option_unset() {
    let signer = signer(&with_params(r#", "optional_params": {"issuer": null}"#));
    assert_eq!(signer.settings.issuer, "litellm");
}

#[test]
fn options_unset_anywhere_take_the_signer_defaults() {
    let signer = signer(&with_params(""));
    let settings = &signer.settings;
    assert_eq!(settings.issuer, "litellm");
    assert_eq!(settings.audience, "mcp");
    assert_eq!(settings.ttl_seconds, 300);
    assert_eq!(settings.channel_token_ttl, 300);
    assert_eq!(
        settings.end_user_claim_sources,
        vec!["token:sub".to_owned(), "litellm:user_id".to_owned()]
    );
    assert!(settings.add_claims.is_empty());
    assert!(settings.set_claims.is_empty());
    assert!(settings.remove_claims.is_empty());
    assert!(settings.required_claims.is_empty());
    assert!(settings.optional_claims.is_empty());
    assert!(settings.allowed_scopes.is_none());
    assert_eq!(settings.timeout, Duration::from_secs(600));
}

#[test]
fn debug_headers_defaults_to_false() {
    assert!(!signer(&with_params("")).settings.debug_headers);
}

#[test]
fn a_zero_ttl_in_optional_params_wins_over_the_top_level() {
    let refused = initialize(&with_params(
        r#", "ttl_seconds": 600, "optional_params": {"ttl_seconds": 0}"#,
    ));
    assert_eq!(error_text(refused), "ttl_seconds must be > 0, got 0");
}

#[rstest]
#[case::issuer(r#""issuer": 5"#, "issuer")]
#[case::ttl_seconds(r#""ttl_seconds": "300""#, "ttl_seconds")]
#[case::add_claims(r#""add_claims": ["a"]"#, "add_claims")]
#[case::allowed_scopes(r#""allowed_scopes": "scope""#, "allowed_scopes")]
fn a_wrong_option_type_names_the_field(#[case] option: &str, #[case] field: &str) {
    let refused = initialize(&with_params(&format!(", {option}")));
    assert_eq!(
        error_text(refused),
        format!("MCPJWTSigner option {field} has an invalid value")
    );
}

#[test]
fn an_option_decode_failure_carries_its_source() {
    let refused = initialize(&with_params(r#", "ttl_seconds": "300""#));
    let error = refused.err().expect("the initializer refused");
    assert_eq!(
        error.to_string(),
        "MCPJWTSigner option ttl_seconds has an invalid value"
    );
    assert!(error.source().is_some());
}

#[test]
fn optional_params_that_is_not_a_mapping_is_refused() {
    let refused = initialize(&with_params(r#", "optional_params": "issuer""#));
    let error = refused.err().expect("the initializer refused");
    assert_eq!(
        error.to_string(),
        "MCPJWTSigner option optional_params has an invalid value"
    );
    assert!(error.source().is_none());
}

#[test]
fn a_negative_timeout_is_refused_with_its_source() {
    let refused = initialize(&with_params(r#", "timeout": -1"#));
    let error = refused.err().expect("the initializer refused");
    assert_eq!(
        error.to_string(),
        "MCPJWTSigner option timeout has an invalid value"
    );
    assert!(error.source().is_some());
}

#[test]
fn the_signer_is_registered_once_and_returned() {
    let mut registry = RecordingRegistry::new();
    let signer = initialize_mcp_jwt_signer(
        &decode(&with_params("")),
        Some("jwt"),
        &mut registry,
        &JwtSignerSlot::new(),
    )
    .expect("the signer builds");
    assert_eq!(registry.callbacks.len(), 1);
    assert!(Arc::ptr_eq(&registry.callbacks[0], &signer));
}

#[test]
fn the_initializer_stores_the_signer_it_returns() {
    let mut registry = RecordingRegistry::new();
    let slot = JwtSignerSlot::new();
    let signer =
        initialize_mcp_jwt_signer(&decode(&with_params("")), Some("jwt"), &mut registry, &slot)
            .expect("the signer builds");
    assert!(Arc::ptr_eq(&slot.get().expect("stored"), &signer));
}

#[test]
fn the_slot_is_empty_until_a_signer_is_stored() {
    assert!(JwtSignerSlot::new().get().is_none());
}

#[test]
fn the_slot_keeps_the_last_signer_stored() {
    let slot = JwtSignerSlot::new();
    let first = generated_signer();
    slot.set(Arc::clone(&first));
    assert!(Arc::ptr_eq(&slot.get().expect("first"), &first));
    let second = generated_signer();
    slot.set(Arc::clone(&second));
    assert!(Arc::ptr_eq(&slot.get().expect("second"), &second));
}

#[test]
fn a_refused_signer_is_neither_registered_nor_stored() {
    let mut registry = RecordingRegistry::new();
    let slot = JwtSignerSlot::new();
    let refused = initialize_mcp_jwt_signer(
        &decode(&with_params(r#", "ttl_seconds": 0"#)),
        Some("jwt"),
        &mut registry,
        &slot,
    );
    assert!(refused.is_err());
    assert!(registry.callbacks.is_empty());
    assert!(slot.get().is_none());
}

#[test]
fn the_process_environment_reaches_the_signer() {
    let mut registry = RecordingRegistry::new();
    registry
        .env
        .insert("MCP_JWT_ISSUER".to_owned(), "env-issuer".to_owned());
    registry
        .env
        .insert("MCP_JWT_AUDIENCE".to_owned(), "env-audience".to_owned());
    let signer = initialize_mcp_jwt_signer(
        &decode(&with_params("")),
        Some("jwt"),
        &mut registry,
        &JwtSignerSlot::new(),
    )
    .expect("the signer builds");
    assert_eq!(signer.settings.issuer, "env-issuer");
    assert_eq!(signer.settings.audience, "env-audience");
}

#[test]
fn a_bad_signing_key_is_a_signer_error() {
    let mut registry = RecordingRegistry::new();
    registry
        .env
        .insert("MCP_JWT_SIGNING_KEY".to_owned(), "not a pem".to_owned());
    let refused = initialize_mcp_jwt_signer(
        &decode(&with_params("")),
        Some("jwt"),
        &mut registry,
        &JwtSignerSlot::new(),
    );
    assert!(error_text(refused).contains("does not hold a PEM document"));
}

#[test]
fn the_mode_is_checked_before_the_signer_is_built() {
    let mut registry = RecordingRegistry::new();
    registry
        .env
        .insert("MCP_JWT_SIGNING_KEY".to_owned(), "not a pem".to_owned());
    let params = decode(r#"{"guardrail": "mcp_jwt_signer", "mode": "pre_call"}"#);
    let refused =
        initialize_mcp_jwt_signer(&params, Some("jwt"), &mut registry, &JwtSignerSlot::new());
    assert_eq!(
        error_text(refused),
        "MCPJWTSigner guardrail 'jwt' has mode='pre_call' but must use mode='pre_mcp_call'. JWT injection only fires for MCP tool calls."
    );
}

#[test]
fn the_registry_entries_use_the_wire_name_and_signer_type() {
    assert_eq!(WIRE_NAME, "mcp_jwt_signer");
    assert_eq!(SIGNER_TYPE_NAME, "MCPJWTSigner");
}
