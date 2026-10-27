#![cfg(test)]

use super::{PromptInfo, PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations};
use crate::decisions::Json;
use rstest::rstest;

#[test]
fn prompt_spec_defaults_environment_and_prompt_type() {
    let spec: PromptSpec = serde_json::from_str(
        r#"{"prompt_id":"welcome","litellm_params":{"prompt_integration":"custom"}}"#,
    )
    .expect("valid prompt spec");
    assert_eq!(spec.environment, "development");
    assert_eq!(spec.prompt_info.prompt_type, "config");
    assert_eq!(spec.prompt_info.environment, "development");
}

#[test]
fn prompt_info_injects_prompt_type_when_missing() {
    let info: PromptInfo =
        serde_json::from_str(r#"{"environment":"production"}"#).expect("valid prompt info");
    assert_eq!(info.prompt_type, "config");
    assert_eq!(info.environment, "production");
}

#[test]
fn prompt_records_keep_extra_fields() {
    let params: PromptLiteLLMParams =
        serde_json::from_str(r#"{"prompt_integration":"dotprompt","future_option":true}"#)
            .expect("valid prompt params");
    assert_eq!(params.extra.get("future_option"), Some(&Json::Bool(true)));
}

#[rstest]
#[case::dotprompt(SupportedPromptIntegrations::DotPrompt, "dotprompt")]
#[case::langfuse(SupportedPromptIntegrations::Langfuse, "langfuse")]
#[case::custom(SupportedPromptIntegrations::Custom, "custom")]
#[case::bitbucket(SupportedPromptIntegrations::Bitbucket, "bitbucket")]
#[case::gitlab(SupportedPromptIntegrations::Gitlab, "gitlab")]
#[case::generic(
    SupportedPromptIntegrations::GenericPromptManagement,
    "generic_prompt_management"
)]
#[case::phoenix(SupportedPromptIntegrations::ArizePhoenix, "arize_phoenix")]
fn supported_integrations_use_litellm_wire_names(
    #[case] integration: SupportedPromptIntegrations,
    #[case] name: &str,
) {
    assert_eq!(
        serde_json::to_string(&integration).unwrap(),
        format!("\"{name}\"")
    );
}
