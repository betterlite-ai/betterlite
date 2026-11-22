#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn vertex_returns_the_vertex_video_config() {
    let selected = get_provider_video_config(None, ProviderId::VertexAi).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::VertexAi));
}

#[test]
fn vertex_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::VertexAi.class_name(), "VertexAIVideoConfig");
}
