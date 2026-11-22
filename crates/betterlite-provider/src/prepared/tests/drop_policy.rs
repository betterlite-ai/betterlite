#![cfg(test)]

use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_models::controls::call_params::DropParams;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use rstest::rstest;
use secrecy::SecretString;

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
