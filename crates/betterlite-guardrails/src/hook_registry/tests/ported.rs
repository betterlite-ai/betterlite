use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::tls::VerifySetting;

use super::*;
use crate::hook_registry::kinds::{
    initialize_alice, initialize_cato_networks, initialize_dynamoai, initialize_enkryptai,
};

#[test]
fn alice_args_copy_their_fields() {
    let params = test_params(wire::ALICE);
    let args = initialize_alice(&params, Some("alice-guard"));
    assert_eq!(args.base.guardrail_name, "alice-guard");
    assert_eq!(
        args.base.api_base.as_deref(),
        Some("https://guard.example/v1")
    );
    assert_eq!(exposed(args.base.api_key.as_ref()), Some("test-key"));
    assert_eq!(args.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(
        args.base.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(args.base.default_on);
    assert_eq!(args.base.timeout, Some(7.5_f64));
}

#[test]
fn alice_keeps_an_explicit_fail_open() {
    let params = params_from(
        r#"{"guardrail":"alice","mode":"pre_call","unreachable_fallback":"fail_open"}"#,
    );
    let args = initialize_alice(&params, Some("alice-guard"));
    assert_eq!(args.unreachable_fallback, UnreachableFallback::FailOpen);
    assert!(args.base.api_key.is_none());
}

#[test]
fn cato_networks_reads_ssl_verify() {
    let params =
        params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":false}"#);
    let args = initialize_cato_networks(&params, Some("cato-guard")).expect("args");
    assert_eq!(args.ssl_verify, Some(VerifySetting::Bool(false)));
    assert_eq!(args.inspect_embeddings, None);
    assert_eq!(args.base.guardrail_name, "cato-guard");
}

#[test]
fn cato_networks_accepts_a_ca_path() {
    let params =
        params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":"/ca.pem"}"#);
    let args = initialize_cato_networks(&params, Some("cato-guard")).expect("args");
    assert_eq!(
        args.ssl_verify,
        Some(VerifySetting::Path("/ca.pem".to_owned()))
    );
}

#[test]
fn cato_networks_rejects_a_ssl_verify_number() {
    let params = params_from(r#"{"guardrail":"cato_networks","mode":"pre_call","ssl_verify":7}"#);
    let error =
        initialize_cato_networks(&params, Some("cato-guard")).expect_err("invalid ssl_verify");
    assert_eq!(
        error.to_string(),
        "the guardrail parameter ssl_verify has an invalid value"
    );
}

#[test]
fn dynamoai_args_copy_their_fields() {
    let params = test_params(wire::DYNAMOAI);
    let args = initialize_dynamoai(&params, Some("dynamo-guard"));
    assert_eq!(args.base.guardrail_name, "dynamo-guard");
    assert_eq!(
        args.base.api_base.as_deref(),
        Some("https://guard.example/v1")
    );
    assert_eq!(exposed(args.base.api_key.as_ref()), Some("test-key"));
    assert_eq!(
        args.base.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(args.base.default_on);
    assert_eq!(args.base.timeout, Some(7.5_f64));
}

#[test]
fn enkryptai_defaults_the_undeclared_params() {
    let params = params_from(r#"{"guardrail":"enkryptai","mode":"pre_call"}"#);
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert_eq!(args.policy_name, None);
    assert_eq!(args.deployment_name, None);
    assert!(args.detectors.is_none());
    assert!(args.block_on_violation);
}

#[test]
fn enkryptai_copies_the_declared_params() {
    let params = params_from(
        r#"{"guardrail":"enkryptai","mode":"pre_call","policy_name":"policy","deployment_name":"deployment","detectors":{"toxicity":true},"block_on_violation":false}"#,
    );
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert_eq!(args.policy_name.as_deref(), Some("policy"));
    assert_eq!(args.deployment_name.as_deref(), Some("deployment"));
    assert_eq!(args.detectors.as_ref().map(BTreeMap::len), Some(1));
    assert!(!args.block_on_violation);
}

#[test]
fn enkryptai_reads_a_null_block_on_violation_as_the_default() {
    let params =
        params_from(r#"{"guardrail":"enkryptai","mode":"pre_call","block_on_violation":null}"#);
    let args = initialize_enkryptai(&params, Some("enkrypt-guard")).expect("args");
    assert!(args.block_on_violation);
}
