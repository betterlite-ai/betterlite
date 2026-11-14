#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::{ExposeSecret as _, SecretString};

use crate::purview_dlp_evaluation::hook_methods::{CONFIG_MODEL, get_supported_event_hooks};
use crate::purview_dlp_evaluation::{
    GUARDRAIL_PROVIDER, MicrosoftPurviewDLPGuardrail, PurviewDlpGuardrailParams,
};

fn params(
    purview_app_name: Option<&str>,
    user_id_field: Option<&str>,
) -> PurviewDlpGuardrailParams {
    PurviewDlpGuardrailParams {
        guardrail_name: "purview-dlp".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("secret-1"),
        purview_app_name: purview_app_name.map(str::to_owned),
        user_id_field: user_id_field.map(str::to_owned),
        timeout: None,
    }
}

fn build(
    purview_app_name: Option<&str>,
    user_id_field: Option<&str>,
) -> MicrosoftPurviewDLPGuardrail {
    MicrosoftPurviewDLPGuardrail::new(
        params(purview_app_name, user_id_field),
        reqwest::Client::default(),
    )
}

#[test]
fn config_model_is_none() {
    assert_eq!(CONFIG_MODEL, None);
}

#[test]
fn supported_event_hooks_are_pre_post_logging_in_order() {
    assert_eq!(
        get_supported_event_hooks(),
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::LoggingOnly,
        ]
    );
}

#[test]
fn supported_event_hooks_serialize_to_litellms_wire_names() {
    assert_eq!(
        serde_json::to_string(&get_supported_event_hooks()).unwrap(),
        r#"["pre_call","post_call","logging_only"]"#
    );
}

#[test]
fn constructor_registers_the_supported_hooks() {
    assert_eq!(
        build(None, None).supported_event_hooks,
        vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::LoggingOnly,
        ]
    );
}

#[test]
fn constructor_keeps_the_given_guardrail_name_and_base_fields() {
    let guardrail = build(None, None);
    assert_eq!(guardrail.guardrail_name, "purview-dlp");
    assert_eq!(guardrail.base.tenant_id, "tenant-1");
    assert_eq!(guardrail.base.client_id, "client-1");
    assert_eq!(guardrail.base.client_secret.expose_secret(), "secret-1");
}

#[test]
fn constructor_defaults_the_app_name_and_user_field() {
    let guardrail = build(None, None);
    assert_eq!(guardrail.base.purview_app_name, "LiteLLM");
    assert_eq!(guardrail.base.user_id_field, "user_id");
}

#[test]
fn constructor_keeps_an_explicit_app_name_and_user_field() {
    let guardrail = build(Some("PurviewApp"), Some("entra_id"));
    assert_eq!(guardrail.base.purview_app_name, "PurviewApp");
    assert_eq!(guardrail.base.user_id_field, "entra_id");
}

#[test]
fn constructor_defaults_the_timeout_to_the_httpx_default() {
    assert_eq!(build(None, None).base.timeout, Duration::from_secs(600));
}

#[test]
fn constructor_keeps_a_given_timeout() {
    let mut given = params(None, None);
    given.timeout = Some(Duration::from_secs(12));
    let guardrail = MicrosoftPurviewDLPGuardrail::new(given, reqwest::Client::default());
    assert_eq!(guardrail.base.timeout, Duration::from_secs(12));
}

#[test]
fn guardrail_provider_is_microsoft_purview() {
    assert_eq!(GUARDRAIL_PROVIDER, "microsoft_purview");
}
