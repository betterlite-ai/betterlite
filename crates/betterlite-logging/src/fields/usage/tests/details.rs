use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::project::usage_json;

use super::{decode, decode_object};

#[rstest]
#[case::null_counters_are_dropped(
    r#"{"prompt_tokens_details":{"web_search_requests":null,"character_count":3}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"character_count":3}"#
)]
#[case::set_counters_are_kept(
    r#"{"prompt_tokens_details":{"web_search_requests":2,"query_count":null}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"web_search_requests":2}"#
)]
#[case::cache_write_alias_fills_from_the_set_one(
    r#"{"prompt_tokens_details":{"cache_write_tokens":null,"cache_creation_tokens":4}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"cache_write_tokens":4,"cache_creation_tokens":4}"#
)]
#[case::null_cache_write_is_not_written(
    r#"{"prompt_tokens_details":{"cache_write_tokens":null}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null}"#
)]
#[case::cached_tokens_details_keeps_three_keys(
    r#"{"prompt_tokens_details":{"cached_tokens_details":{"text_tokens":1,"extra":2}}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"cached_tokens_details":{"text_tokens":1,"audio_tokens":null,"image_tokens":null}}"#
)]
#[case::cache_creation_details_keeps_two_keys(
    r#"{"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_5m_input_tokens":1,"extra":2}}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"cache_creation_token_details":{"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":null}}"#
)]
#[case::unknown_prompt_key_is_kept(
    r#"{"prompt_tokens_details":{"custom":9}}"#,
    "prompt_tokens_details",
    r#"{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"custom":9}"#
)]
#[case::unknown_completion_key_is_kept(
    r#"{"completion_tokens_details":{"custom":9}}"#,
    "completion_tokens_details",
    r#"{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":null,"rejected_prediction_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"custom":9}"#
)]
#[case::reasoning_detail_is_not_overwritten(
    r#"{"completion_tokens":7,"reasoning_tokens":3,"completion_tokens_details":{"reasoning_tokens":5}}"#,
    "completion_tokens_details",
    r#"{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":5,"rejected_prediction_tokens":null,"text_tokens":4,"image_tokens":null,"video_tokens":null}"#
)]
#[case::text_detail_is_not_derived_again(
    r#"{"completion_tokens":7,"reasoning_tokens":3,"completion_tokens_details":{"text_tokens":1}}"#,
    "completion_tokens_details",
    r#"{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":3,"rejected_prediction_tokens":null,"text_tokens":1,"image_tokens":null,"video_tokens":null}"#
)]
#[case::image_and_audio_come_off_the_derived_text(
    r#"{"completion_tokens":10,"reasoning_tokens":3,"completion_tokens_details":{"image_tokens":2,"audio_tokens":1}}"#,
    "completion_tokens_details",
    r#"{"accepted_prediction_tokens":null,"audio_tokens":1,"reasoning_tokens":3,"rejected_prediction_tokens":null,"text_tokens":4,"image_tokens":2,"video_tokens":null}"#
)]
#[case::no_completion_total_derives_no_text(
    r#"{"reasoning_tokens":3}"#,
    "completion_tokens_details",
    r#"{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":3,"rejected_prediction_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null}"#
)]
fn logging_fields_behavior_tests_behavior(
    #[case] mapping: &str,
    #[case] key: &str,
    #[case] expected: &str,
) {
    let usage = usage_from_mapping(&decode_object(mapping)).unwrap();
    let Json::Object(fields) = usage_json(&usage) else {
        panic!("the usage projection is not an object")
    };
    assert_eq!(fields.get(key), Some(&decode(expected)));
}
