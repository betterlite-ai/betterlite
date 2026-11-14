#![cfg(test)]

use std::any::TypeId;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::base_builder;
use reqwest::Client;
use secrecy::{ExposeSecret as _, SecretString};

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::model_armor::config::{
    DEFAULT_LOCATION, ModelArmorConfigModel, ModelArmorGuardrail, ModelArmorGuardrailConfigModel,
    ModelArmorParams, SUPPORTED_EVENT_HOOKS,
};

static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

fn client() -> Client {
    base_builder().build().expect("test client")
}

fn counting_client() -> Client {
    FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
    client()
}

fn panicking_client() -> Client {
    panic!("the factory must not run when a handler is given");
}

fn switches() -> GuardrailSwitches {
    GuardrailSwitches {
        use_native_during_call_hook: false,
        use_native_lifecycle_hooks: false,
        records_own_information: false,
        uses_apply_guardrail_interface: false,
        deny_text_on_error: false,
    }
}

fn settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some("armor".to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: switches(),
    }
}

fn params() -> ModelArmorParams {
    ModelArmorParams {
        template_id: Some("template".to_owned()),
        project_id: Some("project".to_owned()),
        location: None,
        credentials: None,
        api_endpoint: None,
        sanitize_error_detail: None,
        async_handler: Some(client()),
        build_client: client,
        settings: settings(),
        strict_modes: true,
    }
}

fn build(params: ModelArmorParams) -> ModelArmorGuardrail {
    ModelArmorGuardrail::new(params).expect("guardrail")
}

fn litellm_params(source: &str) -> LitellmParams {
    decode_litellm_params(source.as_bytes()).expect("litellm params")
}

#[test]
fn new_stores_the_configuration_and_defaults_the_location() {
    let guardrail = build(ModelArmorParams {
        credentials: Some(SecretString::from("service-account-json")),
        api_endpoint: Some("https://modelarmor.example.test".to_owned()),
        ..params()
    });
    assert_eq!(guardrail.template_id.as_deref(), Some("template"));
    assert_eq!(guardrail.project_id.as_deref(), Some("project"));
    assert_eq!(guardrail.location, DEFAULT_LOCATION);
    assert_eq!(guardrail.location, "us-central1");
    assert_eq!(
        guardrail
            .credentials
            .as_ref()
            .expect("credentials")
            .expose_secret(),
        "service-account-json"
    );
    assert_eq!(
        guardrail.api_endpoint.as_deref(),
        Some("https://modelarmor.example.test")
    );
    assert!(guardrail.sanitize_error_detail);
}

#[test]
fn new_keeps_an_explicit_location_and_the_sanitize_choice() {
    let guardrail = build(ModelArmorParams {
        location: Some("europe-west3".to_owned()),
        sanitize_error_detail: Some(false),
        ..params()
    });
    assert_eq!(guardrail.location, "europe-west3");
    assert!(!guardrail.sanitize_error_detail);
    let explicit_true = build(ModelArmorParams {
        sanitize_error_detail: Some(true),
        ..params()
    });
    assert!(explicit_true.sanitize_error_detail);
}

#[test]
fn new_defaults_the_event_hook_and_the_supported_hooks() {
    let guardrail = build(params());
    let settings = &guardrail.base.settings;
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "during_call".to_owned(),
            "post_call".to_owned(),
        ]))
    );
    assert_eq!(
        settings.supported_event_hooks,
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    );
    assert_eq!(settings.guardrail_name.as_deref(), Some("armor"));
    assert!(settings.switches.use_native_lifecycle_hooks);
    assert!(settings.switches.uses_apply_guardrail_interface);
}

#[test]
fn new_keeps_a_configured_event_hook_and_supported_hooks() {
    let event_hook = LitellmParamsMode::One("pre_call".to_owned());
    let supported = vec![GuardrailEventHooks::PreCall];
    let mut configured = settings();
    configured.event_hook = Some(event_hook.clone());
    configured.supported_event_hooks = Some(supported.clone());
    let guardrail = build(ModelArmorParams {
        settings: configured,
        ..params()
    });
    assert_eq!(guardrail.base.settings.event_hook, Some(event_hook));
    assert_eq!(
        guardrail.base.settings.supported_event_hooks,
        Some(supported)
    );
}

#[test]
fn new_rejects_an_event_hook_outside_the_supported_list() {
    let mut configured = settings();
    configured.event_hook = Some(LitellmParamsMode::One("post_mcp_call".to_owned()));
    let error = ModelArmorGuardrail::new(ModelArmorParams {
        settings: configured,
        ..params()
    })
    .err()
    .expect("unsupported hook");
    assert!(matches!(
        error,
        GuardrailBaseError::UnsupportedEventHook { .. }
    ));
}

#[test]
fn new_warns_and_continues_when_strict_modes_is_off() {
    let event_hook = LitellmParamsMode::One("post_mcp_call".to_owned());
    let mut configured = settings();
    configured.event_hook = Some(event_hook.clone());
    let guardrail = build(ModelArmorParams {
        settings: configured,
        strict_modes: false,
        ..params()
    });
    assert_eq!(guardrail.base.settings.event_hook, Some(event_hook));
}

#[test]
fn new_uses_the_given_async_handler() {
    let guardrail = build(ModelArmorParams {
        async_handler: Some(client()),
        build_client: panicking_client,
        ..params()
    });
    assert_eq!(guardrail.location, DEFAULT_LOCATION);
}

#[test]
fn new_builds_a_client_when_no_handler_is_given() {
    FACTORY_CALLS.store(0, Ordering::SeqCst);
    let guardrail = build(ModelArmorParams {
        async_handler: None,
        build_client: counting_client,
        ..params()
    });
    assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(guardrail.location, DEFAULT_LOCATION);
}

#[test]
fn update_litellm_params_applies_an_explicit_false() {
    let mut guardrail = build(params());
    guardrail.update_litellm_params(&litellm_params(
        r#"{"guardrail":"model_armor","mode":"pre_call","sanitize_error_detail":false}"#,
    ));
    assert!(!guardrail.sanitize_error_detail);
}

#[test]
fn update_litellm_params_keeps_sanitizing_on_a_null_update() {
    let mut guardrail = build(ModelArmorParams {
        sanitize_error_detail: Some(false),
        ..params()
    });
    guardrail.update_litellm_params(&litellm_params(
        r#"{"guardrail":"model_armor","mode":"pre_call","sanitize_error_detail":null}"#,
    ));
    assert!(guardrail.sanitize_error_detail);
}

#[test]
fn update_litellm_params_updates_the_base_settings() {
    let event_hook = LitellmParamsMode::One("pre_call".to_owned());
    let mut configured = settings();
    configured.event_hook = Some(event_hook.clone());
    let mut guardrail = build(ModelArmorParams {
        settings: configured,
        ..params()
    });
    guardrail.update_litellm_params(&litellm_params(
        r#"{"guardrail":"model_armor","mode":"post_call","mask_request_content":true,"timeout":1.5}"#,
    ));
    assert!(guardrail.base.settings.mask_request_content);
    assert_eq!(guardrail.base.settings.timeout, Some(1.5_f64));
    assert_eq!(guardrail.base.settings.event_hook, Some(event_hook));
}

#[test]
fn get_supported_event_hooks_lists_the_six_model_armor_events() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::DuringMcpCall,
            GuardrailEventHooks::LoggingOnly,
        ]
    );
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).expect("serialize"),
        r#"["pre_call","during_call","post_call","pre_mcp_call","during_mcp_call","logging_only"]"#
    );
}

#[test]
fn get_config_model_is_the_model_armor_config_model() {
    assert_eq!(
        TypeId::of::<ModelArmorConfigModel>(),
        TypeId::of::<ModelArmorGuardrailConfigModel>()
    );
}

#[test]
fn config_model_defaults_the_flags_and_the_ui_name() {
    let model: ModelArmorGuardrailConfigModel = serde_json::from_str("{}").expect("config model");
    assert_eq!(model.template_id, None);
    assert_eq!(model.project_id, None);
    assert_eq!(model.location, None);
    assert!(model.credentials.is_none());
    assert_eq!(model.api_endpoint, None);
    assert!(model.fail_on_error);
    assert!(model.sanitize_error_detail);
    assert!(model.optional_params.is_none());
    assert_eq!(
        ModelArmorGuardrailConfigModel::ui_friendly_name(),
        "Google Cloud Model Armor"
    );
}

#[test]
fn config_model_reads_every_field_and_treats_null_as_the_default() {
    let model: ModelArmorGuardrailConfigModel = serde_json::from_str(
        r#"{"template_id":"t","project_id":"p","location":"europe-west3","credentials":"creds.json","api_endpoint":"https://example.test","fail_on_error":false,"sanitize_error_detail":null,"optional_params":{"k":1}}"#,
    )
    .expect("config model");
    assert_eq!(model.template_id.as_deref(), Some("t"));
    assert_eq!(model.project_id.as_deref(), Some("p"));
    assert_eq!(model.location.as_deref(), Some("europe-west3"));
    assert_eq!(
        model
            .credentials
            .as_ref()
            .expect("credentials")
            .expose_secret(),
        "creds.json"
    );
    assert_eq!(model.api_endpoint.as_deref(), Some("https://example.test"));
    assert!(!model.fail_on_error);
    assert!(model.sanitize_error_detail);
    assert!(model.optional_params.is_some());
}
