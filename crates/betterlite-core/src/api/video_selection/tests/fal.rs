#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn fal_returns_the_fal_video_config() {
    let selected = get_provider_video_config(None, ProviderId::FalAi).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::FalAi));
}

#[test]
fn fal_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::FalAi.class_name(), "FalAIVideoConfig");
}
