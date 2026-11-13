use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use rstest::rstest;

use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::project::usage_json;
use crate::fields::usage::{NativeResponseUsage, Usage, transform_usage_objects};

use super::{decode, decode_object};

fn usage(mapping: &str) -> Usage {
    usage_from_mapping(&decode_object(mapping)).unwrap()
}

#[test]
fn a_responses_usage_passes_through_the_endpoint_projection() {
    let converted = usage(r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}"#);
    assert_eq!(
        transform_usage_objects(NativeResponseUsage::Responses(&converted)),
        Some(converted)
    );
}

#[rstest]
#[case::duration_has_no_tokens(r#"{"type":"duration","seconds":1.5}"#, "null")]
#[case::tokens_project_to_chat_usage(
    r#"{"type":"tokens","input_tokens":10,"output_tokens":3,"total_tokens":999,"input_token_details":{"audio_tokens":7,"text_tokens":3}}"#,
    r#"{"prompt_tokens":10,"completion_tokens":3,"total_tokens":999,"completion_tokens_details":null,"prompt_tokens_details":{"audio_tokens":7,"cached_tokens":null,"text_tokens":3,"image_tokens":null,"video_tokens":null}}"#
)]
fn logging_fields_behavior_tests_behavior(#[case] native: &str, #[case] expected: &str) {
    let transcription: TranscriptionUsage = serde_json::from_str(native).unwrap();
    let projected = transform_usage_objects(NativeResponseUsage::Transcription(&transcription));
    let actual = projected.map_or_else(
        || decode("null"),
        |projected_usage| usage_json(&projected_usage),
    );
    assert_eq!(actual, decode(expected));
}
