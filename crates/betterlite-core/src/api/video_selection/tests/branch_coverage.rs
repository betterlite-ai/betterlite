#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::error::VideoSelectionError;
use super::{VideoConfigTag, get_provider_video_config};

#[rstest]
#[case::openai(ProviderId::Openai, VideoConfigTag::OpenAi)]
#[case::azure(ProviderId::Azure, VideoConfigTag::Azure)]
#[case::gemini(ProviderId::Gemini, VideoConfigTag::Gemini)]
#[case::vertex_ai(ProviderId::VertexAi, VideoConfigTag::VertexAi)]
#[case::runwayml(ProviderId::Runwayml, VideoConfigTag::RunwayMl)]
#[case::fal_ai(ProviderId::FalAi, VideoConfigTag::FalAi)]
fn every_argument_free_branch_selects_its_config(
    #[case] provider: ProviderId,
    #[case] expected: VideoConfigTag,
) {
    assert_eq!(
        get_provider_video_config(Some("sora-2"), provider).unwrap(),
        Some(expected)
    );
}

#[rstest]
#[case::vllm(ProviderId::Vllm)]
#[case::azure_ai(ProviderId::AzureAi)]
#[case::vertex_ai_beta(ProviderId::VertexAiBeta)]
#[case::anthropic(ProviderId::Anthropic)]
#[case::bedrock(ProviderId::Bedrock)]
fn providers_without_a_video_config_select_none(#[case] provider: ProviderId) {
    assert_eq!(
        get_provider_video_config(Some("sora-2"), provider).unwrap(),
        None
    );
}

#[test]
fn hosted_vllm_takes_the_model_and_edenai_errors() {
    assert_eq!(
        get_provider_video_config(Some("Qwen/Qwen2.5-VL"), ProviderId::HostedVllm).unwrap(),
        Some(VideoConfigTag::HostedVllm(Some(
            "Qwen/Qwen2.5-VL".to_owned()
        )))
    );
    assert!(matches!(
        get_provider_video_config(None, ProviderId::Edenai),
        Err(VideoSelectionError::Unavailable { provider }) if provider == "edenai"
    ));
}
