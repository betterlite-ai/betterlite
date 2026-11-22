#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::error::VideoSelectionError;
use super::{VideoConfigTag, get_provider_video_config};

#[test]
fn signature_takes_an_optional_model_and_a_provider() {
    let with_model: Result<Option<VideoConfigTag>, VideoSelectionError> =
        get_provider_video_config(Some("sora-2"), ProviderId::Openai);
    assert_eq!(with_model, Ok(Some(VideoConfigTag::OpenAi)));

    let without_model: Result<Option<VideoConfigTag>, VideoSelectionError> =
        get_provider_video_config(None, ProviderId::Openai);
    assert_eq!(without_model, Ok(Some(VideoConfigTag::OpenAi)));
}
