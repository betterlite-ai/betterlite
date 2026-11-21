#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn runway_returns_the_runway_video_config() {
    let selected = get_provider_video_config(None, ProviderId::Runwayml).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::RunwayMl));
}

#[test]
fn runway_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::RunwayMl.class_name(), "RunwayMLVideoConfig");
}
