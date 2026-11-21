#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn hosted_vllm_carries_the_model() {
    let selected =
        get_provider_video_config(Some("Qwen/Qwen2.5-VL"), ProviderId::HostedVllm).unwrap();
    assert_eq!(
        selected,
        Some(VideoConfigTag::HostedVllm(Some(
            "Qwen/Qwen2.5-VL".to_owned()
        )))
    );
}

#[test]
fn hosted_vllm_keeps_a_missing_model() {
    let selected = get_provider_video_config(None, ProviderId::HostedVllm).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::HostedVllm(None)));
}

#[test]
fn hosted_vllm_video_config_is_the_litellm_class() {
    assert_eq!(
        VideoConfigTag::HostedVllm(None).class_name(),
        "HostedVLLMVideoConfig"
    );
}
