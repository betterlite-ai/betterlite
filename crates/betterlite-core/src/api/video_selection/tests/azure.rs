#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn azure_returns_the_azure_video_config() {
    let selected = get_provider_video_config(None, ProviderId::Azure).unwrap();
    assert_eq!(selected, Some(VideoConfigTag::Azure));
}

#[test]
fn azure_video_config_is_the_litellm_class() {
    assert_eq!(VideoConfigTag::Azure.class_name(), "AzureVideoConfig");
}
