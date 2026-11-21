#![cfg(test)]

use betterlite_models::baseline_history::usage::{
    CacheCreationTokenDetails, PromptTokensDetails, Usage,
};
use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use super::request_usage;

fn usage() -> Usage {
    let mut prompt_extra = Extra::default();
    prompt_extra.insert("audio_tokens".to_owned(), Json::Number(Number::from(7_u8)));
    prompt_extra.insert("image_tokens".to_owned(), Json::Number(Number::from(2_u8)));
    prompt_extra.insert("video_tokens".to_owned(), Json::Number(Number::from(3_u8)));
    let mut extra = Extra::default();
    extra.insert(
        "completion_tokens_details".to_owned(),
        serde_json::from_str(r#"{"reasoning_tokens":4,"text_tokens":8}"#).unwrap(),
    );
    Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(5),
            cached_tokens: Some(2),
            cache_creation_tokens: Some(3),
            cache_creation_token_details: Some(CacheCreationTokenDetails {
                ephemeral_5m_input_tokens: Some(1),
                ephemeral_1h_input_tokens: Some(2),
            }),
            cache_write_tokens: Some(3),
            extra: prompt_extra,
        }),
        cache_read_input_tokens: Some(2),
        cache_creation_input_tokens: Some(3),
        extra,
    }
}

#[test]
fn request_usage_maps_the_estimate_usage_and_its_token_details() {
    let request = request_usage(&usage());
    assert_eq!(
        (
            request.usage.prompt_tokens,
            request.usage.completion_tokens,
            request.usage.total_tokens,
        ),
        (10, 5, 15)
    );
    assert_eq!(
        (
            request.usage.cache_read_input_tokens,
            request.usage.cache_creation_input_tokens,
        ),
        (2, 3)
    );
    let prompt = request.usage.prompt_tokens_details.unwrap();
    assert_eq!(
        (
            prompt.cached_tokens,
            prompt.cache_creation_tokens,
            prompt.cache_write_tokens,
            prompt.text_tokens,
        ),
        (2, 3, 3, 5)
    );
    assert_eq!(
        (
            prompt.audio_tokens,
            prompt.image_tokens,
            prompt.video_tokens
        ),
        (7, 2, 3)
    );
    let writes = request.cache_creation_token_details.unwrap();
    assert_eq!(
        (
            writes.ephemeral_5m_input_tokens,
            writes.ephemeral_1h_input_tokens,
        ),
        (Some(1), Some(2))
    );
    let completion = request.usage.completion_tokens_details.unwrap();
    assert_eq!(
        (completion.reasoning_tokens, completion.text_tokens),
        (Some(4), Some(8))
    );
}
