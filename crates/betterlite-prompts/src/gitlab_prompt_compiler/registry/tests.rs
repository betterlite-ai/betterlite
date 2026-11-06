#![cfg(test)]

use betterlite_models::prompt_spec::{
    PromptInfo, PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations,
};
use indexmap::IndexMap;
use rstest::rstest;

use super::prompt_initializer_registry;
use crate::gitlab_client::tests::FakeTransport;

fn params() -> PromptLiteLLMParams {
    PromptLiteLLMParams {
        prompt_id: None,
        prompt_integration: "gitlab".to_owned(),
        api_base: None,
        api_key: None,
        provider_specific_query_params: None,
        ignore_model: false,
        ignore_optional_params: false,
        dotprompt_content: None,
        extra: IndexMap::from([(
            "gitlab_config".to_owned(),
            serde_json::from_str(r#"{"project": "g/r", "access_token": "t"}"#).unwrap(),
        )]),
    }
}

fn spec() -> PromptSpec {
    PromptSpec {
        prompt_id: "chat/hi".to_owned(),
        litellm_params: params(),
        prompt_info: PromptInfo::default(),
        created_at: None,
        updated_at: None,
        version: None,
        environment: "development".to_owned(),
        created_by: None,
    }
}

#[tokio::test]
async fn the_registry_serves_the_gitlab_initializer() {
    let initializer =
        prompt_initializer_registry::<FakeTransport>(&SupportedPromptIntegrations::Gitlab)
            .expect("gitlab is registered");
    let spec = spec();

    let manager = initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .unwrap();

    assert_eq!(manager.prompt_id.as_deref(), Some("chat/hi"));
}

#[rstest]
#[case::dotprompt(SupportedPromptIntegrations::DotPrompt)]
#[case::langfuse(SupportedPromptIntegrations::Langfuse)]
#[case::custom(SupportedPromptIntegrations::Custom)]
#[case::bitbucket(SupportedPromptIntegrations::Bitbucket)]
#[case::generic(SupportedPromptIntegrations::GenericPromptManagement)]
#[case::phoenix(SupportedPromptIntegrations::ArizePhoenix)]
fn other_integrations_are_not_served_by_the_gitlab_registry(
    #[case] integration: SupportedPromptIntegrations,
) {
    assert!(prompt_initializer_registry::<FakeTransport>(&integration).is_none());
}
