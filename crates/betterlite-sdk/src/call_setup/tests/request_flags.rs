use rstest::rstest;

use super::support::request;
use crate::call_setup::request_flags::is_async_request;
use crate::call_setup::request_flags::is_streaming_request;

#[rstest]
#[case::acompletion("acompletion")]
#[case::aembedding("aembedding")]
#[case::aimg_generation("aimg_generation")]
#[case::amoderation("amoderation")]
#[case::atext_completion("atext_completion")]
#[case::atranscription("atranscription")]
#[case::arerank("arerank")]
#[case::arealtime("_arealtime")]
#[case::acreate_batch("acreate_batch")]
#[case::acreate_fine_tuning_job("acreate_fine_tuning_job")]
#[case::aresponses("aresponses")]
fn request_flag_async_keys_are_async(#[case] key: &str) {
    let request = request(&format!(r#"{{"{key}":true}}"#));

    assert!(is_async_request(&request, false));
}

#[rstest]
#[case::false_value(r#"{"acompletion":false}"#)]
#[case::text_value(r#"{"acompletion":"true"}"#)]
#[case::other_key(r#"{"aimage_generation":true}"#)]
fn request_flag_non_true_values_are_sync(#[case] source: &str) {
    assert!(!is_async_request(&request(source), false));
}

#[test]
fn request_flag_pass_through_is_async() {
    assert!(is_async_request(&request("{}"), true));
}

#[test]
fn request_flag_empty_request_is_sync() {
    assert!(!is_async_request(&request("{}"), false));
}

#[rstest]
#[case::stream_true(r#"{"stream":true}"#, "completion")]
#[case::genai_stream("{}", "generate_content_stream")]
#[case::agenai_stream("{}", "agenerate_content_stream")]
fn request_flag_streaming_inputs(#[case] source: &str, #[case] call_type: &str) {
    assert!(is_streaming_request(&request(source), call_type));
}

#[rstest]
#[case::stream_false(r#"{"stream":false}"#)]
#[case::stream_text(r#"{"stream":"true"}"#)]
#[case::stream_null(r#"{"stream":null}"#)]
fn request_flag_non_streaming_inputs(#[case] source: &str) {
    assert!(!is_streaming_request(&request(source), "completion"));
}

#[test]
fn request_flag_generate_content_is_not_a_stream() {
    assert!(!is_streaming_request(&request("{}"), "generate_content"));
}
