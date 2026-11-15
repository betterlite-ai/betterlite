use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::hiddenlayer::evaluation::tests::fixtures::{self, text, text_inputs};
use crate::hiddenlayer::evaluation::tests::{server, tokens};

use super::*;

fn params(extra: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "hiddenlayer", "mode": "pre_call"{extra}}}"#);
    serde_json::from_str(&body).expect("params")
}

fn instance(params: &LitellmParams) -> HiddenlayerGuardrailInstance {
    initialize_guardrail(
        params,
        Some("hiddenlayer-guard"),
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
        Uuid::new_v4,
    )
    .expect("an instance")
}

#[test]
fn absent_version_dispatches_to_v1() {
    let guardrail = instance(&params(r#", "api_base": "http://127.0.0.1:1""#));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V1(_)));
}

#[test]
fn null_version_dispatches_to_v1() {
    let guardrail = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": null"#,
    ));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V1(_)));
}

#[test]
fn version_zero_dispatches_to_v1() {
    let guardrail = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 0"#,
    ));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V1(_)));
}

#[test]
fn version_one_dispatches_to_v1() {
    let guardrail = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 1"#,
    ));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V1(_)));
}

#[test]
fn version_two_dispatches_to_v2() {
    let guardrail = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 2"#,
    ));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V2(_)));
}

#[test]
fn version_three_dispatches_to_v2() {
    let guardrail = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 3"#,
    ));
    assert!(matches!(guardrail, HiddenlayerGuardrailInstance::V2(_)));
}

#[test]
fn a_wrong_version_type_is_a_typed_initializer_error() {
    let wrong = initialize_guardrail(
        &params(r#", "version": "two""#),
        Some("hiddenlayer-guard"),
        &fixtures::environment(),
        fixtures::client(),
        Arc::new(tokens::ScriptedTokens::new(&[])),
        Uuid::new_v4,
    );
    let Err(EvaluationError::Initializer(error)) = wrong else {
        panic!("a rejected version");
    };
    assert!(error.to_string().contains("version"), "{error}");
}

#[test]
fn evaluation_args_read_every_parameter() {
    let params = params(
        r#", "api_id": "api-id", "api_key": "api-key", "api_base": "http://api.example",
            "auth_url": "http://auth.example", "timeout": 5.0, "default_on": true"#,
    );
    let args = evaluation_args(&params, Some("hiddenlayer-guard")).expect("args");
    assert_eq!(args.guardrail_name.as_deref(), Some("hiddenlayer-guard"));
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert_eq!(args.api_id.as_deref(), Some("api-id"));
    assert_eq!(
        args.api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("api-key")
    );
    assert_eq!(args.api_base.as_deref(), Some("http://api.example"));
    assert_eq!(args.auth_url.as_deref(), Some("http://auth.example"));
    assert_eq!(args.timeout, Some(5.0));
    assert!(args.default_on);
}

#[test]
fn evaluation_args_default_the_optionals() {
    let args = evaluation_args(&params(""), None).expect("args");
    assert_eq!(args.guardrail_name.as_deref(), Some(""));
    assert_eq!(args.api_id, None);
    assert_eq!(args.api_key, None);
    assert_eq!(args.api_base, None);
    assert_eq!(args.auth_url, None);
    assert_eq!(args.timeout, None);
    assert!(!args.default_on);
}

#[test]
fn config_model_and_event_hooks_differ_by_version() {
    let v1 = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 1"#,
    ));
    let v2 = instance(&params(
        r#", "api_base": "http://127.0.0.1:1", "version": 2"#,
    ));
    assert_eq!(v1.get_config_model(), Some(config_model()));
    assert_eq!(v2.get_config_model(), Some(config_model()));
    assert_eq!(
        v1.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(v2.get_supported_event_hooks(), None);
}

#[tokio::test]
async fn contract_delegates_to_v1() {
    let server = server::start(vec![server::reply(
        r#"{"evaluation": {"action": "Allow"}}"#,
    )]);
    let params = params(&format!(r#", "api_base": "{}", "version": 1"#, server.base));
    let guardrail = instance(&params);
    let mut data = BTreeMap::new();
    let evaluated = guardrail
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    assert_eq!(evaluated.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn contract_maps_a_v1_block_to_the_http_object() {
    let server = server::start(vec![server::reply(
        r#"{"evaluation": {"action": "Block", "threat_level": "high"},
            "analysis": [{"name": "prompt_injection", "detected": true}]}"#,
    )]);
    let params = params(&format!(r#", "api_base": "{}", "version": 1"#, server.base));
    let guardrail = instance(&params);
    let mut data = BTreeMap::new();
    let failure = guardrail
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("blocked");
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("an object failure");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text(crate::hiddenlayer::evaluation::VIOLATED_POLICY))
    );
    assert_eq!(
        detail.get("block_reasons"),
        Some(&GuardrailValue::Array(vec![text("prompt_injection")]))
    );
}

#[tokio::test]
async fn contract_maps_a_provider_failure_to_a_500() {
    let server = server::start(vec![server::reply_with(403, &[], "denied")]);
    let params = params(&format!(r#", "api_base": "{}", "version": 1"#, server.base));
    let guardrail = instance(&params);
    let mut data = BTreeMap::new();
    let failure = guardrail
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("failed");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("a plain failure");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("403"), "{detail}");
}

#[tokio::test]
async fn contract_delegates_to_v2() {
    let server = server::start(vec![server::reply(r#"{"messages": []}"#)]);
    let params = params(&format!(r#", "api_base": "{}", "version": 2"#, server.base));
    let guardrail = instance(&params);
    let mut data = BTreeMap::new();
    let evaluated = guardrail
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("evaluated");
    assert_eq!(evaluated.texts, Some(Vec::new()));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        server.requests()[0].path,
        "/detection/v2/request-evaluations"
    );
}
