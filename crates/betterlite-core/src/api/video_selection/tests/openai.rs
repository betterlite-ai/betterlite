#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn openai_returns_the_openai_video_config() {
    let selected = get_provider_video_config(None, ProviderId::Openai).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::OpenAi));
}

#[test]
fn openai_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::OpenAi.class_name(), "OpenAIVideoConfig");
}
