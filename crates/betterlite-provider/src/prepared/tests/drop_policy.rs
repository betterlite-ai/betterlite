#![cfg(test)]

use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_models::controls::call_params::DropParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::params::ParameterError;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::operation::OperationError;
use crate::prepared::body::parameter_policy;
use crate::prepared::deployment::Deployment;

fn chat_deployment(drop_policy: DeploymentDropPolicy) -> ChatDeployment {
    let mut chat = ChatDeployment::resolve(
        &ModelDeployment {
            model_name: String::from("g"),
            litellm_params: LiteLlmParams {
                ssl_verify: None,
                model: Some(String::from("openai/up")),
                api_base: Some(String::from("http://127.0.0.1:9/v1")),
                api_key: Some(SecretString::from("key")),
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            },
            model_info: ModelInfo::default(),
        },
        &OpenAiDefaults::default(),
    )
    .expect("the deployment resolves");
    chat.drop_policy = drop_policy;
    chat
}

#[rstest]
#[case::flag_copied(DeploymentDropPolicy { drop_params: Some(false), additional_drop_params: Vec::new() })]
#[case::list_copied(DeploymentDropPolicy { drop_params: None, additional_drop_params: vec![String::from("temperature"), String::from("top_p")] })]
#[case::default_unset(DeploymentDropPolicy::default())]
fn from_chat_drop_policy_cases(#[case] drop_policy: DeploymentDropPolicy) {
    let deployment = Deployment::from_chat(&chat_deployment(drop_policy.clone()));
    assert_eq!(
        deployment.additional_drop_params,
        drop_policy.additional_drop_params
    );
    match drop_policy.drop_params {
        Some(flag) => assert!(matches!(
            deployment.drop_params,
            Some(DropParams::Flag(given)) if given == flag
        )),
        None => assert!(deployment.drop_params.is_none()),
    }
}

fn fields(text: &str) -> IndexMap<String, JsonValue> {
    serde_json::from_str(text).expect("the parameter map parses")
}

// The router spreads the deployment's litellm_params under the request's kwargs, so a silent request takes the deployment's values.
#[test]
fn parameter_policy_reads_the_deployment_when_the_request_is_silent() {
    let deployment = Deployment::from_chat(&chat_deployment(DeploymentDropPolicy {
        drop_params: Some(true),
        additional_drop_params: vec![String::from("temperature")],
    }));
    let (absent_policy, absent_call) =
        parameter_policy(&fields("{}"), &deployment).expect("the policy resolves");
    assert!(absent_call, "the deployment's flag is the call's flag");
    assert!(absent_policy.drop_params);
    assert_eq!(
        absent_policy.additional_drop_params,
        vec![String::from("temperature")]
    );
    assert!(absent_policy.allowed_openai_params.is_empty());
    let nulled = fields(r#"{"drop_params":null,"additional_drop_params":null}"#);
    let (null_policy, null_call) =
        parameter_policy(&nulled, &deployment).expect("the policy resolves");
    assert!(null_call, "a null flag reads as unset");
    assert_eq!(
        null_policy.additional_drop_params,
        vec![String::from("temperature")]
    );
}

// A request that sends the flag answers for the call, and its lists replace the deployment's.
#[test]
fn parameter_policy_prefers_the_request_values() {
    let deployment = Deployment::from_chat(&chat_deployment(DeploymentDropPolicy {
        drop_params: Some(true),
        additional_drop_params: vec![String::from("temperature")],
    }));
    let given = fields(
        r#"{"drop_params":true,"additional_drop_params":["top_p"],"allowed_openai_params":["response_format"]}"#,
    );
    let (policy, call_drop) = parameter_policy(&given, &deployment).expect("the policy resolves");
    assert!(call_drop, "the request's own true flag is the call's flag");
    assert!(policy.drop_params);
    assert_eq!(policy.additional_drop_params, vec![String::from("top_p")]);
    assert_eq!(
        policy.allowed_openai_params,
        vec![String::from("response_format")]
    );
    let refused = fields(r#"{"drop_params":false}"#);
    let (refused_policy, refused_call) =
        parameter_policy(&refused, &deployment).expect("the policy resolves");
    assert!(
        !refused_call,
        "the request's own false flag is the call's flag"
    );
    assert!(
        !refused_policy.drop_params,
        "the request's false turns the deployment's true off"
    );
    assert_eq!(
        refused_policy.additional_drop_params,
        vec![String::from("temperature")]
    );
}

// _check_valid_arg reads litellm.drop_params and the call's flag as an OR, so the global one keeps the drop on.
#[test]
fn parameter_policy_keeps_the_global_drop_flag() {
    let mut deployment = Deployment::from_chat(&chat_deployment(DeploymentDropPolicy::default()));
    deployment.global_drop_params = true;
    let (policy, call_drop) = parameter_policy(&fields(r#"{"drop_params":false}"#), &deployment)
        .expect("the policy resolves");
    assert!(!call_drop, "the call's own flag stays false");
    assert!(policy.drop_params, "the global flag keeps the drop on");
}

// A request value the flag parser cannot read answers the 400 the server arm gives a Parameters error.
#[test]
fn parameter_policy_refuses_a_wrong_typed_flag() {
    let deployment = Deployment::from_chat(&chat_deployment(DeploymentDropPolicy::default()));
    let error = parameter_policy(&fields(r#"{"drop_params":"loud"}"#), &deployment)
        .expect_err("a text flag is refused");
    assert!(matches!(
        error,
        OperationError::Parameters {
            source: ParameterError::Encode { .. }
        }
    ));
}
