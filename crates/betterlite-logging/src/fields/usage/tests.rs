#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::parse::usage_from_mapping;
use super::project::usage_json;

mod errors;
mod from_response;
mod native;
mod projections;

fn decode_object(encoded: &str) -> IndexMap<String, Json> {
    match serde_json::from_str(encoded) {
        Ok(Json::Object(fields)) => fields,
        Ok(_) | Err(_) => panic!("expected a JSON object: {encoded}"),
    }
}

fn decode(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

#[rstest]
#[case::empty("{}", r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null}"#)]
#[case::ordinary(
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}"#,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
#[case::extras(
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"custom":7}"#,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null,"custom":7}"#
)]
#[case::prompt_details(
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"prompt_tokens_details":{"cached_tokens":1,"text_tokens":2}}"#,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":{"audio_tokens":null,"cached_tokens":1,"text_tokens":2,"image_tokens":null,"video_tokens":null}}"#
)]
#[case::completion_details(
    r#"{"completion_tokens":3,"completion_tokens_details":{"reasoning_tokens":1}}"#,
    r#"{"prompt_tokens":0,"completion_tokens":3,"total_tokens":0,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":1,"rejected_prediction_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":null}"#
)]
#[case::cache_read_alias(
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"cache_read_input_tokens":4}"#,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":{"audio_tokens":null,"cached_tokens":4,"text_tokens":null,"image_tokens":null,"video_tokens":null},"cache_read_input_tokens":4}"#
)]
#[case::cache_write_alias(
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"cache_creation_input_tokens":7}"#,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":{"audio_tokens":null,"cached_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null,"cache_write_tokens":7,"cache_creation_tokens":7},"cache_creation_input_tokens":7}"#
)]
#[case::reasoning_derives_text(
    r#"{"prompt_tokens":2,"completion_tokens":7,"total_tokens":9,"reasoning_tokens":3}"#,
    r#"{"prompt_tokens":2,"completion_tokens":7,"total_tokens":9,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":3,"rejected_prediction_tokens":null,"text_tokens":4,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":null}"#
)]
#[case::server_tool_use(
    r#"{"server_tool_use":{"web_search_requests":1}}"#,
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null,"server_tool_use":{"web_search_requests":1,"tool_search_requests":null,"browser_open_requests":null}}"#
)]
#[case::cost(
    r#"{"cost":0.25}"#,
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null,"cost":0.25}"#
)]
#[case::exponent_cost(
    r#"{"cost":1.5e-06}"#,
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null,"cost":0.0000015}"#
)]
#[case::exponent_integers(
    r#"{"prompt_tokens":1e3,"completion_tokens":2e2,"total_tokens":1.2e3}"#,
    r#"{"prompt_tokens":1000,"completion_tokens":200,"total_tokens":1200,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
fn logging_fields_behavior_tests_behavior(#[case] mapping: &str, #[case] expected: &str) {
    let usage = usage_from_mapping(&decode_object(mapping)).unwrap();
    assert_eq!(usage_json(&usage), decode(expected));
}
