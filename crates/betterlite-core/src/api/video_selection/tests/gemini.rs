#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn gemini_returns_the_gemini_video_config() {
    let selected = get_provider_video_config(None, ProviderId::Gemini).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::Gemini));
}

#[test]
fn gemini_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::Gemini.class_name(), "GeminiVideoConfig");
}
