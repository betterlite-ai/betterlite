#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::super::{FileProviderConfig, file_config_selection};

#[rstest]
#[case::gemini_empty_model(
    "",
    ProviderId::Gemini,
    FileProviderConfig::GoogleAiStudio,
    "GoogleAIStudioFilesHandler"
)]
#[case::gemini_named_model(
    "ignored",
    ProviderId::Gemini,
    FileProviderConfig::GoogleAiStudio,
    "GoogleAIStudioFilesHandler"
)]
#[case::vertex_ai_empty_model(
    "",
    ProviderId::VertexAi,
    FileProviderConfig::VertexAi,
    "VertexAIFilesConfig"
)]
#[case::vertex_ai_named_model(
    "ignored",
    ProviderId::VertexAi,
    FileProviderConfig::VertexAi,
    "VertexAIFilesConfig"
)]
#[case::bedrock_empty_model(
    "",
    ProviderId::Bedrock,
    FileProviderConfig::Bedrock,
    "BedrockFilesConfig"
)]
#[case::bedrock_named_model(
    "ignored",
    ProviderId::Bedrock,
    FileProviderConfig::Bedrock,
    "BedrockFilesConfig"
)]
#[case::manus_empty_model("", ProviderId::Manus, FileProviderConfig::Manus, "ManusFilesConfig")]
#[case::manus_named_model(
    "ignored",
    ProviderId::Manus,
    FileProviderConfig::Manus,
    "ManusFilesConfig"
)]
#[case::anthropic_empty_model(
    "",
    ProviderId::Anthropic,
    FileProviderConfig::Anthropic,
    "AnthropicFilesConfig"
)]
#[case::anthropic_named_model(
    "ignored",
    ProviderId::Anthropic,
    FileProviderConfig::Anthropic,
    "AnthropicFilesConfig"
)]
#[case::mistral_empty_model(
    "",
    ProviderId::Mistral,
    FileProviderConfig::Mistral,
    "MistralFilesConfig"
)]
#[case::mistral_named_model(
    "ignored",
    ProviderId::Mistral,
    FileProviderConfig::Mistral,
    "MistralFilesConfig"
)]
#[case::xai_empty_model("", ProviderId::Xai, FileProviderConfig::Xai, "XAIFilesConfig")]
#[case::xai_named_model("ignored", ProviderId::Xai, FileProviderConfig::Xai, "XAIFilesConfig")]
fn each_registered_provider_selects_its_config(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] expected: FileProviderConfig,
    #[case] class_name: &str,
) {
    let selected = file_config_selection(model, provider).unwrap();
    assert_eq!(selected, expected);
    assert_eq!(selected.class_name(), class_name);
}

#[rstest]
#[case::openai_empty_model("", ProviderId::Openai)]
#[case::openai_named_model("ignored", ProviderId::Openai)]
#[case::azure_empty_model("", ProviderId::Azure)]
#[case::azure_named_model("ignored", ProviderId::Azure)]
#[case::hosted_vllm_empty_model("", ProviderId::HostedVllm)]
#[case::hosted_vllm_named_model("ignored", ProviderId::HostedVllm)]
#[case::litellm_proxy_empty_model("", ProviderId::LitellmProxy)]
#[case::litellm_proxy_named_model("ignored", ProviderId::LitellmProxy)]
#[case::cohere_empty_model("", ProviderId::Cohere)]
#[case::cohere_named_model("ignored", ProviderId::Cohere)]
#[case::vertex_ai_beta("", ProviderId::VertexAiBeta)]
#[case::azure_ai("", ProviderId::AzureAi)]
#[case::anthropic_text("", ProviderId::AnthropicText)]
#[case::cohere_chat("", ProviderId::CohereChat)]
fn providers_without_a_files_config_select_none(#[case] model: &str, #[case] provider: ProviderId) {
    assert_eq!(file_config_selection(model, provider), None);
}
