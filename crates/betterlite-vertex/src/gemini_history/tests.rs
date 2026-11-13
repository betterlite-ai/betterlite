#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::future::{Future, ready};
use std::iter::repeat_n;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::error::GeminiToolInvokesError;
use betterlite_prompts::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use betterlite_prompts::gemini_tool_invokes::signatures::DUMMY_THOUGHT_SIGNATURE;
use betterlite_prompts::gemini_tool_results::error::GeminiToolResultsError;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media::limits::{MAX_MESSAGE_PARTS, MAX_MESSAGES};
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use crate::gemini_history::content::{MAX_THINKING_BYTES, Role, decode_thinking_part};
use crate::gemini_history::convert::{History, HistoryWarning, convert_messages_with_history};
use crate::gemini_history::error::HistoryError;
use crate::gemini_media::error::{MediaError, MediaField};

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

fn parse_messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn parse_json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn encode<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn convert_with(
    runtime: &Runtime,
    messages: &[Message],
    model: Option<&str>,
) -> Result<History, HistoryError> {
    runtime.block_on(convert_messages_with_history(messages, model, &NoFetch))
}

fn converted_messages(messages: &[Message], model: Option<&str>) -> History {
    let runtime = Builder::new_current_thread().build().unwrap();
    convert_with(&runtime, messages, model).unwrap()
}

fn converted(source: &str, model: Option<&str>) -> History {
    converted_messages(&parse_messages(source), model)
}

const TOOL_ROUND_TRIP: &str = r#"[{"role":"user","content":"q"},{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"c1","content":"res"}]"#;

const THINKING_TOOL_CALL: &str = r#"[{"role":"user","content":"q"},{"role":"assistant","content":"calling","thinking_blocks":[{"type":"thinking","thinking":"{\"functionCall\": {\"name\": \"f\", \"args\": {\"a\": 1}}}","signature":"sig"}],"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}]}]"#;

fn assert_parts(source: &str, model: Option<&str>, expected: &str) {
    let history = converted(source, model);
    let resolved = expected.replace("SIGNATURE", DUMMY_THOUGHT_SIGNATURE);
    assert_eq!(encode(&history.contents), parse_json(&resolved));
}

fn assert_model_turn(assistant: &str, expected_parts: &str) {
    assert_parts(
        &format!(r#"[{{"role":"user","content":"q"}},{assistant}]"#),
        None,
        &format!(
            r#"[{{"role":"user","parts":[{{"text":"q"}}]}},{{"role":"model","parts":{expected_parts}}}]"#
        ),
    );
}

fn error_for(messages: &[Message], model: Option<&str>) -> HistoryError {
    let runtime = Builder::new_current_thread().build().unwrap();
    convert_with(&runtime, messages, model).unwrap_err()
}

fn error_of(source: &str, model: Option<&str>) -> HistoryError {
    error_for(&parse_messages(source), model)
}

fn assert_field_error(error: &HistoryError, field: &str) {
    assert_eq!(error.status(), 400);
    assert!(
        error.to_string().contains(field),
        "{error} does not name {field}"
    );
}

#[test]
fn merges_consecutive_user_and_system_messages() {
    assert_parts(
        r#"[{"role":"system","content":"sys"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"sys"},{"text":"u"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn merges_consecutive_assistant_messages() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"one"},{"role":"assistant","content":[{"type":"text","text":"two"},{"type":"text","text":""}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"one"},{"text":"two"},{"text":""}]}]"#,
    );
}

#[test]
fn preserves_empty_string_content() {
    assert_parts(
        r#"[{"role":"user","content":""},{"role":"assistant","content":""}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":""}]},{"role":"model","parts":[{"text":""}]}]"#,
    );
}

#[test]
fn ignores_empty_text_list_elements_in_user_turns() {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"text","text":""},{"type":"text","text":"u"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"u"}]}]"#,
    );
}

#[test]
fn keeps_empty_text_list_elements_in_assistant_turns() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":[{"type":"text","text":""},{"type":"text","text":"a"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":""},{"text":"a"}]}]"#,
    );
}

#[test]
fn adds_a_space_to_a_media_only_user_turn() {
    let history = converted(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj"}}]}]"#,
        None,
    );
    assert_eq!(
        encode(&history.contents),
        parse_json(
            r#"[{"role":"user","parts":[{"inline_data":{"data":"YWJj","mime_type":"image/png"}},{"text":" "}]}]"#
        )
    );
    assert_eq!(history.warnings, vec![HistoryWarning::BlankText]);
}

#[test]
fn turns_an_empty_history_into_one_space_turn() {
    let history = converted("[]", None);
    assert_eq!(
        encode(&history.contents),
        parse_json(r#"[{"role":"user","parts":[{"text":" "}]}]"#)
    );
    assert_eq!(history.warnings, vec![HistoryWarning::EmptyHistory]);
}

#[test]
fn a_user_turn_with_only_unknown_parts_is_empty() {
    let history = converted(
        r#"[{"role":"user","content":[{"type":"video_url","video_url":{"url":"x"}}]}]"#,
        None,
    );
    assert_eq!(
        encode(&history.contents),
        parse_json(r#"[{"role":"user","parts":[{"text":" "}]}]"#)
    );
    assert_eq!(history.warnings, vec![HistoryWarning::EmptyHistory]);
}

#[test]
fn groups_adjacent_tool_results_into_one_user_turn() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"calling","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"c2","type":"function","function":{"name":"g","arguments":"{}"}}]},{"role":"tool","tool_call_id":"c1","content":"r1"},{"role":"tool","tool_call_id":"c2","content":"r2"},{"role":"user","content":"thanks"}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"calling"},{"function_call":{"name":"f","args":{}}},{"function_call":{"name":"g","args":{}}}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"r1"}}},{"function_response":{"name":"g","response":{"content":"r2"}}}]},{"role":"user","parts":[{"text":"thanks"}]}]"#,
    );
}

#[test]
fn matches_a_tool_result_after_a_plain_assistant_message() {
    let expected = r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"calling"},{"function_call":{"name":"f","args":{},"id":"c1"},"thoughtSignature":"SIGNATURE"},{"text":"plain"}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"res"},"id":"c1"}}]}]"#
        .replace("SIGNATURE", DUMMY_THOUGHT_SIGNATURE);
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"calling","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"assistant","content":"plain"},{"role":"tool","tool_call_id":"c1","content":"res"}]"#,
        Some("gemini-3-pro-preview"),
        &expected,
    );
}

#[rstest]
#[case::older_model_omits_ids(
    None,
    r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"function_call":{"name":"f","args":{}}}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"res"}}}]}]"#
)]
#[case::gemini_two_omits_ids(
    Some("gemini-2.5-flash"),
    r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"function_call":{"name":"f","args":{}}}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"res"}}}]}]"#
)]
#[case::gemini_three_forwards_ids(
    Some("gemini-3-pro-preview"),
    r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"function_call":{"name":"f","args":{},"id":"c1"},"thoughtSignature":"SIGNATURE"}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"res"},"id":"c1"}}]}]"#
)]
fn forwards_tool_call_ids_only_for_newer_models(
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    assert_parts(TOOL_ROUND_TRIP, model, expected);
}

#[rstest]
#[case::empty("", r#"[{"thought":true,"text":""}]"#)]
#[case::text("why", r#"[{"thought":true,"text":"why"}]"#)]
fn replays_reasoning_content_as_a_thought_part(#[case] reasoning: &str, #[case] expected: &str) {
    assert_model_turn(
        &format!(r#"{{"role":"assistant","content":null,"reasoning_content":"{reasoning}"}}"#),
        expected,
    );
}

#[rstest]
#[case::unprefixed_format(
    "wav",
    r#"[{"role":"user","parts":[{"inline_data":{"data":"YWJj","mime_type":"audio/wav"}},{"text":" "}]}]"#
)]
#[case::prefixed_format(
    "audio/mp3",
    r#"[{"role":"user","parts":[{"inline_data":{"data":"YWJj","mime_type":"audio/mp3"}},{"text":" "}]}]"#
)]
fn converts_input_audio_to_inline_data(#[case] format: &str, #[case] expected: &str) {
    let source = format!(
        r#"[{{"role":"user","content":[{{"type":"input_audio","input_audio":{{"data":"YWJj","format":"{format}"}}}}]}}]"#
    );
    assert_parts(&source, None, expected);
}

#[rstest]
#[case::missing_data(r#"{"format":"wav"}"#)]
#[case::missing_format(r#"{"data":"YWJj"}"#)]
#[case::null_data(r#"{"data":null,"format":"wav"}"#)]
fn skips_an_audio_part_without_data_or_format(#[case] audio: &str) {
    let source = format!(
        r#"[{{"role":"user","content":[{{"type":"input_audio","input_audio":{audio}}},{{"type":"text","text":"u"}}]}}]"#
    );
    assert_parts(&source, None, r#"[{"role":"user","parts":[{"text":"u"}]}]"#);
}

#[test]
fn keeps_file_format_before_mime_type_and_content_type() {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"data:image/png;base64,YWJj","format":"image/webp","mime_type":"image/bmp","content_type":"image/gif"}}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"inline_data":{"data":"YWJj","mime_type":"image/webp"}},{"text":" "}]}]"#,
    );
}

#[rstest]
#[case::older_model_drops_the_resolution(
    "gemini-2.5-flash",
    r#"[{"role":"user","parts":[{"file_data":{"mime_type":"video/mp4","file_uri":"gs://bucket/clip.mp4"},"video_metadata":{"fps":2,"startOffset":"1s"}},{"text":" "}]}]"#
)]
#[case::newer_model_keeps_the_resolution(
    "gemini-3-pro-preview",
    r#"[{"role":"user","parts":[{"file_data":{"mime_type":"video/mp4","file_uri":"gs://bucket/clip.mp4"},"media_resolution":{"level":"MEDIA_RESOLUTION_HIGH"},"video_metadata":{"fps":2,"startOffset":"1s"}},{"text":" "}]}]"#
)]
fn forwards_video_metadata_and_detail_from_a_file_block(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"gs://bucket/clip.mp4","detail":"high","video_metadata":{"fps":2,"start_offset":"1s"}}}]}]"#,
        Some(model),
        expected,
    );
}

#[rstest]
#[case::with_a_model(
    Some("gemini-2.5-flash"),
    r#"[{"role":"user","parts":[{"file_data":{"mime_type":"video/mp4","file_uri":"gs://b/clip.mp4"},"video_metadata":{"fps":1}},{"text":" "}]}]"#
)]
#[case::without_a_model(
    None,
    r#"[{"role":"user","parts":[{"file_data":{"mime_type":"video/mp4","file_uri":"gs://b/clip.mp4"}},{"text":" "}]}]"#
)]
fn ignores_video_metadata_keys_other_than_fps_and_offsets(
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"gs://b/clip.mp4","video_metadata":{"fps":1,"note":"x"}}}]}]"#,
        model,
        expected,
    );
}

#[test]
fn rejects_video_metadata_that_is_not_an_object() {
    let error = error_of(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"gs://b/clip.mp4","video_metadata":"fps"}}]}]"#,
        None,
    );
    assert_field_error(&error, "video_metadata");
}

#[test]
fn takes_the_detail_of_an_image_url_object_for_newer_models() {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"gs://bucket/pic.png","detail":"ultra_high","video_metadata":{"fps":2}}}]}]"#,
        Some("gemini-3-pro-preview"),
        r#"[{"role":"user","parts":[{"file_data":{"mime_type":"image/png","file_uri":"gs://bucket/pic.png"},"media_resolution":{"level":"MEDIA_RESOLUTION_ULTRA_HIGH"}},{"text":" "}]}]"#,
    );
}

#[test]
fn converts_an_image_url_given_as_a_plain_string() {
    assert_parts(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,YWJj"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"inline_data":{"data":"YWJj","mime_type":"image/png"}},{"text":" "}]}]"#,
    );
}

#[test]
fn skips_generated_images_that_are_not_a_list() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","images":{"image_url":{"url":"data:image/png;base64,YWJj"}}}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn replays_a_signed_thinking_block() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","thinking_blocks":[{"type":"thinking","thinking":"{\"text\": \"reason\", \"thought\": true}","signature":"sig"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"thoughtSignature":"sig","text":"reason","thought":true},{"text":"a"}]}]"#,
    );
}

#[rstest]
#[case::plain_text(
    "not json",
    r#"[{"thoughtSignature":"sig","text":"not json"},{"text":"a"}]"#
)]
#[case::array_json(
    "[\"a\"]",
    r#"[{"thoughtSignature":"sig","text":"[\"a\"]"},{"text":"a"}]"#
)]
#[case::signature_key_collision(
    "{\"thoughtSignature\": \"x\", \"text\": \"t\"}",
    r#"[{"thoughtSignature":"sig","text":"{\"thoughtSignature\": \"x\", \"text\": \"t\"}"},{"text":"a"}]"#
)]
fn falls_back_to_the_signed_text_for_unusable_thinking_json(
    #[case] thinking: &str,
    #[case] expected: &str,
) {
    let quoted = serde_json::to_string(thinking).unwrap();
    let source = format!(
        r#"[{{"role":"user","content":"q"}},{{"role":"assistant","content":"a","thinking_blocks":[{{"type":"thinking","thinking":{quoted},"signature":"sig"}}]}}]"#
    );
    let history = converted(&source, None);
    assert_eq!(
        encode(&history.contents),
        parse_json(&format!(
            r#"[{{"role":"user","parts":[{{"text":"q"}}]}},{{"role":"model","parts":{expected}}}]"#
        ))
    );
}

#[rstest]
#[case::missing_signature(
    r#"{"role":"assistant","content":"a","thinking_blocks":[{"type":"thinking","thinking":"{}"}]}"#
)]
#[case::null_signature(r#"{"role":"assistant","content":"a","thinking_blocks":[{"type":"thinking","thinking":"{}","signature":null}]}"#)]
#[case::missing_thinking(
    r#"{"role":"assistant","content":"a","thinking_blocks":[{"type":"thinking","signature":"x"}]}"#
)]
#[case::redacted(r#"{"role":"assistant","content":"a","thinking_blocks":[{"type":"redacted_thinking","data":"x"}]}"#)]
#[case::empty_list(r#"{"role":"assistant","content":"a","thinking_blocks":[]}"#)]
fn skips_thinking_blocks_that_carry_no_signed_thinking(#[case] assistant: &str) {
    let source = format!(r#"[{{"role":"user","content":"q"}},{assistant}]"#);
    assert_parts(
        &source,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn replays_generated_images() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","images":[{"image_url":{"url":"data:image/png;base64,YWJj","detail":"auto"},"index":0,"type":"image_url"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]}]"#,
    );
}

#[rstest]
#[case::format_first(r#""format":"image/gif","mime_type":"image/webp""#, "image/gif")]
#[case::mime_type_second(r#""mime_type":"image/webp","content_type":"image/bmp""#, "image/webp")]
#[case::content_type_last(r#""content_type":"image/bmp""#, "image/bmp")]
fn takes_the_mime_hint_and_detail_of_a_generated_image(
    #[case] hints: &str,
    #[case] mime_type: &str,
) {
    let assistant = format!(
        r#"{{"role":"assistant","content":"a","images":[{{"image_url":{{"url":"data:image/png;base64,YWJj","detail":"high",{hints}}},"index":0,"type":"image_url"}}]}}"#
    );
    let expected = format!(
        r#"[{{"role":"user","parts":[{{"text":"q"}}]}},{{"role":"model","parts":[{{"text":"a"}},{{"inline_data":{{"data":"YWJj","mime_type":"{mime_type}"}},"media_resolution":{{"level":"MEDIA_RESOLUTION_HIGH"}}}}]}}]"#
    );
    assert_parts(
        &format!(r#"[{{"role":"user","content":"q"}},{assistant}]"#),
        Some("gemini-3-pro-preview"),
        &expected,
    );
}

#[rstest]
#[case::no_url(r#"{"image_url":{}}"#)]
#[case::empty_url(r#"{"image_url":{"url":""}}"#)]
fn skips_generated_images_without_a_url(#[case] image: &str) {
    let source = format!(
        r#"[{{"role":"user","content":"q"}},{{"role":"assistant","content":"a","images":[{image}]}}]"#
    );
    assert_parts(
        &source,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn skips_a_generated_image_whose_image_url_is_a_string() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","images":[{"image_url":"data:image/png;base64,YWJj"}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[rstest]
#[case::call_and_response(
    r#"{"tool_type":"google_search","id":"s1","args":{"q":"x"},"thought_signature":"sigs","response":{"r":1},"response_thought_signature":"sigr"}"#,
    r#"{"toolCall":{"toolType":"google_search","id":"s1","args":{"q":"x"}},"thoughtSignature":"sigs"},{"toolResponse":{"id":"s1","response":{"r":1},"toolType":"google_search"},"thoughtSignature":"sigr"}"#
)]
#[case::call_only(
    r#"{"id":"s1"}"#,
    r#"{"toolCall":{"toolType":null,"id":"s1","args":null}}"#
)]
#[case::null_response(
    r#"{"id":"s1","response":null}"#,
    r#"{"toolCall":{"toolType":null,"id":"s1","args":null}},{"toolResponse":{"id":"s1","response":null}}"#
)]
#[case::empty_tool_type_kept(
    r#"{"id":"s1","tool_type":"","response":{}}"#,
    r#"{"toolCall":{"toolType":"","id":"s1","args":null}},{"toolResponse":{"id":"s1","response":{},"toolType":""}}"#
)]
fn replays_server_side_tool_invocations(#[case] invocation: &str, #[case] expected: &str) {
    assert_model_turn(
        &format!(
            r#"{{"role":"assistant","content":"a","provider_specific_fields":{{"server_side_tool_invocations":[{invocation}]}}}}"#
        ),
        &format!(r#"[{{"text":"a"}},{expected}]"#),
    );
}

#[test]
fn keeps_a_text_signature_that_no_tool_carries() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["sig-text"]},"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
        Some("gemini-3-pro-preview"),
        &(r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a","thoughtSignature":"sig-text"},{"function_call":{"name":"f","args":{},"id":"c1"},"thoughtSignature":"SIGNATURE"}]}]"#
            .replace("SIGNATURE", DUMMY_THOUGHT_SIGNATURE)),
    );
}

#[test]
fn drops_a_text_signature_that_a_tool_already_carries() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["sig-tool"]},"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"},"provider_specific_fields":{"thought_signature":"sig-tool"}}]}]"#,
        Some("gemini-3-pro-preview"),
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"function_call":{"name":"f","args":{},"id":"c1"},"thoughtSignature":"sig-tool"}]}]"#,
    );
}

#[test]
fn a_synthetic_tool_signature_does_not_suppress_the_text_signature() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["real"]},"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
        Some("gemini-3-pro-preview"),
        &(r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a","thoughtSignature":"real"},{"function_call":{"name":"f","args":{},"id":"c1"},"thoughtSignature":"SIGNATURE"}]}]"#
            .replace("SIGNATURE", DUMMY_THOUGHT_SIGNATURE)),
    );
}

#[test]
fn collects_a_signature_embedded_in_a_tool_call_id() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["sig-id"]},"tool_calls":[{"id":"c1__thought__sig-id","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
        Some("gemini-2.5-flash"),
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"function_call":{"name":"f","args":{}},"thoughtSignature":"sig-id"}]}]"#,
    );
}

#[test]
fn collects_a_signature_from_a_server_side_invocation() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["sig-server"],"server_side_tool_invocations":[{"id":"s1","thought_signature":"sig-server"}]}}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"},{"toolCall":{"toolType":null,"id":"s1","args":null},"thoughtSignature":"sig-server"}]}]"#,
    );
}

#[test]
fn collects_a_signature_from_a_server_side_response() {
    assert_model_turn(
        r#"{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["sig-resp"],"server_side_tool_invocations":[{"id":"s1","response":{},"response_thought_signature":"sig-resp"}]}}"#,
        r#"[{"text":"a"},{"toolCall":{"toolType":null,"id":"s1","args":null}},{"toolResponse":{"id":"s1","response":{}},"thoughtSignature":"sig-resp"}]"#,
    );
}

#[rstest]
#[case::null_signature(r#"[null,"second"]"#, r#"[{"text":"a","thoughtSignature":null}]"#)]
#[case::empty_signature(r#"["","second"]"#, r#"[{"text":"a","thoughtSignature":""}]"#)]
#[case::first_signature_only(
    r#"["first","second"]"#,
    r#"[{"text":"a","thoughtSignature":"first"}]"#
)]
fn replays_the_first_text_signature_as_given(#[case] signatures: &str, #[case] expected: &str) {
    let source = r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":SIGNATURES}}]"#
        .replace("SIGNATURES", signatures);
    let history = converted(&source, None);
    assert_eq!(
        encode(&history.contents),
        parse_json(&format!(
            r#"[{{"role":"user","parts":[{{"text":"q"}}]}},{{"role":"model","parts":{expected}}}]"#
        ))
    );
}

#[rstest]
#[case::gemini_two_deduplicates(
    Some("gemini-2.5-flash"),
    r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"thoughtSignature":"sig","functionCall":{"name":"f","args":{"a":1}}},{"text":"calling"}]}]"#
)]
#[case::gemini_three_keeps_the_id_bearing_call(
    Some("gemini-3-pro-preview"),
    r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"thoughtSignature":"sig","functionCall":{"name":"f","args":{"a":1}}},{"text":"calling"},{"function_call":{"name":"f","args":{"a":1},"id":"c1"},"thoughtSignature":"SIGNATURE"}]}]"#
)]
fn does_not_repeat_a_tool_call_a_thinking_block_already_carries(
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    assert_parts(THINKING_TOOL_CALL, model, expected);
}

#[rstest]
#[case::missing_image_url(r#"[{"type":"image_url"}]"#, "image_url")]
#[case::null_image_url(r#"[{"type":"image_url","image_url":null}]"#, "image_url")]
#[case::missing_url(r#"[{"type":"image_url","image_url":{}}]"#, "image_url.url")]
#[case::missing_file(r#"[{"type":"file"}]"#, "file")]
#[case::null_file(r#"[{"type":"file","file":null}]"#, "file")]
#[case::missing_file_source(r#"[{"type":"file","file":{}}]"#, "file")]
#[case::missing_audio_field(r#"[{"type":"input_audio"}]"#, "input_audio")]
fn rejects_a_malformed_user_part(#[case] part: &str, #[case] field: &str) {
    let source = format!(r#"[{{"role":"user","content":{part}}}]"#);
    assert_field_error(&error_of(&source, None), field);
}

#[test]
fn skips_a_user_text_part_with_a_null_text() {
    let history = converted(
        r#"[{"role":"user","content":[{"type":"text","text":null},{"type":"text","text":"u"}]}]"#,
        None,
    );
    assert_eq!(
        encode(&history.contents),
        parse_json(r#"[{"role":"user","parts":[{"text":"u"}]}]"#)
    );
}

#[test]
fn skips_assistant_fields_that_are_not_the_declared_shape() {
    let source = r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","reasoning_content":5,"thinking_blocks":{},"images":{},"provider_specific_fields":{"server_side_tool_invocations":["x"]}}]"#;
    assert_parts(
        source,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn drops_a_text_signature_that_is_not_a_scalar() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":[["x"]]}}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"text":"a"}]}]"#,
    );
}

#[test]
fn rejects_a_message_role_the_history_cannot_place() {
    let error = error_of(
        r#"[{"role":"developer","content":"d"},{"role":"user","content":"u"}]"#,
        None,
    );
    assert_field_error(&error, "developer");
    assert!(matches!(error, HistoryError::Invalid(_)));
    assert_eq!(error.error_type(), None);
    assert_eq!(error.param(), None);
    assert_eq!(error.code(), Some("400"));
}

#[test]
fn propagates_the_remote_media_rejection() {
    let error = error_of(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/media"}}]}]"#,
        None,
    );
    assert!(matches!(
        error,
        HistoryError::Media(MediaError::RemoteMediaRequired(MediaField::ImageUrl))
    ));
    assert_eq!(error.error_type(), Some("invalid_request_error"));
    assert_eq!(error.param(), Some("image_url"));
    assert_eq!(error.code(), Some("unsupported_remote_media"));
    let HistoryError::Media(source) = error else {
        panic!("a remote media reference must stay a media error");
    };
    assert_eq!(source.status(), 400);
    assert_eq!(source.param(), Some("image_url"));
    assert_eq!(source.code(), Some("unsupported_remote_media"));
}

#[test]
fn propagates_a_tool_result_without_a_matching_call() {
    let error = error_of(
        r#"[{"role":"user","content":"q"},{"role":"tool","tool_call_id":"c9","content":"x"}]"#,
        None,
    );
    assert!(matches!(
        error,
        HistoryError::ToolResult(GeminiToolResultsError::MissingName)
    ));
}

#[test]
fn caps_the_history_at_the_message_limit() {
    let message: Message = serde_json::from_str(r#"{"role":"user","content":"x"}"#).unwrap();
    let at_limit: Vec<Message> = repeat_n(message.clone(), MAX_MESSAGES).collect();
    converted_messages(&at_limit, None);
    let over_limit: Vec<Message> = repeat_n(message, MAX_MESSAGES + 1).collect();
    assert!(matches!(
        error_for(&over_limit, None),
        HistoryError::TooManyMessages(limit) if limit == MAX_MESSAGES
    ));
}

#[rstest]
#[case::no_branch(r#"{"file_data":"plain-text"}"#, "provided data")]
#[case::inline_without_data_prefix(r#"{"file_data":"base64-payload"}"#, "provided data")]
#[case::named_by_file_id(r#"{"file_id":"plain-id"}"#, "plain-id")]
fn reports_unusable_file_media_as_a_bodyless_bad_request(#[case] file: &str, #[case] named: &str) {
    let source = format!(r#"[{{"role":"user","content":[{{"type":"file","file":{file}}}]}}]"#);
    let error = error_of(&source, None);
    assert!(matches!(
        &error,
        HistoryError::FileMedia {
            source: MediaError::InvalidMedia(_) | MediaError::InvalidInlineData(_),
            ..
        }
    ));
    assert!(error.source().is_some());
    assert_field_error(
        &error,
        &format!("Unable to determine mime type for file: {named}, set this explicitly"),
    );
    assert_eq!(error.error_type(), None);
    assert_eq!(error.param(), None);
    assert_eq!(error.code(), Some("400"));
}

#[test]
fn keeps_an_unusable_image_url_a_media_error() {
    let error = error_of(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"plain-text"}}]}]"#,
        None,
    );
    assert!(matches!(
        error,
        HistoryError::Media(MediaError::InvalidMedia(_))
    ));
    assert_eq!(error.error_type(), Some("invalid_request_error"));
    assert_eq!(error.code(), None);
}

fn with_items(template: &str, item: &str, count: usize) -> Vec<Message> {
    let items = repeat_n(item, count).collect::<Vec<_>>().join(",");
    parse_messages(&template.replace("ITEMS", &items))
}

#[rstest]
#[case::user_content(
    r#"[{"role":"user","content":[ITEMS]}]"#,
    r#"{"type":"text","text":"x"}"#
)]
#[case::assistant_content(
    r#"[{"role":"assistant","content":[ITEMS]}]"#,
    r#"{"type":"text","text":"x"}"#
)]
#[case::thinking_blocks(
    r#"[{"role":"assistant","content":"a","thinking_blocks":[ITEMS]}]"#,
    "1"
)]
#[case::images(r#"[{"role":"assistant","content":"a","images":[ITEMS]}]"#, "1")]
#[case::server_side_invocations(
    r#"[{"role":"assistant","content":"a","provider_specific_fields":{"server_side_tool_invocations":[ITEMS]}}]"#,
    "1"
)]
fn caps_each_message_list_at_the_part_limit(#[case] template: &str, #[case] item: &str) {
    converted_messages(&with_items(template, item, MAX_MESSAGE_PARTS), None);
    let error = error_for(&with_items(template, item, MAX_MESSAGE_PARTS + 1), None);
    assert!(matches!(
        error,
        HistoryError::TooManyParts(limit) if limit == MAX_MESSAGE_PARTS
    ));
    assert_eq!(error.status(), 400);
}

#[test]
fn caps_the_tool_calls_a_text_signature_scan_walks() {
    let call = r#"{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}"#;
    let at_limit = r#"[{"role":"assistant","content":"a","provider_specific_fields":{"thought_signatures":["s"]},"tool_calls":[ITEMS]}]"#;
    converted_messages(&with_items(at_limit, call, MAX_TOOL_CALLS), None);
    let over_limit = at_limit.replace("[ITEMS]", "[1,ITEMS]");
    let error = error_for(&with_items(&over_limit, call, MAX_TOOL_CALLS), None);
    assert!(matches!(
        error,
        HistoryError::ToolInvoke(GeminiToolInvokesError::TooManyToolCalls { limit })
            if limit == MAX_TOOL_CALLS
    ));
    assert_eq!(error.status(), 400);
}

#[test]
fn replays_a_legacy_function_call_and_its_function_result() {
    let history = converted(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"calling","function_call":{"name":"f","arguments":"{}","provider_specific_fields":{"thought_signature":"s"}},"provider_specific_fields":{"thought_signatures":["s"]}},{"role":"function","name":"f","content":"r"}]"#,
        None,
    );
    let text_part = history.contents[1].parts.first().unwrap();
    assert_eq!(
        text_part.get("text"),
        Some(&Json::Text("calling".to_owned()))
    );
    assert!(!text_part.contains_key("thoughtSignature"));
    let result_turn = &history.contents[2];
    assert_eq!(result_turn.role, Role::User);
    assert_eq!(result_turn.parts.len(), 1);
    assert!(
        result_turn
            .parts
            .first()
            .unwrap()
            .contains_key("function_response")
    );
}

#[test]
fn nests_tool_media_inside_the_function_response_turn() {
    assert_parts(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"c1","content":[{"type":"text","text":"t"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj"}}]}]"#,
        None,
        r#"[{"role":"user","parts":[{"text":"q"}]},{"role":"model","parts":[{"function_call":{"name":"f","args":{}}}]},{"role":"user","parts":[{"function_response":{"name":"f","response":{"content":"t"},"parts":[{"inline_data":{"data":"YWJj","mime_type":"image/png"}}]}}]}]"#,
    );
}

#[test]
fn a_second_conversion_leaves_the_input_messages_unchanged() {
    let source = r#"[{"role":"user","content":[{"type":"text","text":"q"},{"type":"image_url","image_url":{"url":"data:image/png;base64,YWJj"}}]},{"role":"assistant","content":"calling","reasoning_content":"why","thinking_blocks":[{"type":"thinking","thinking":"{}","signature":"sig"}],"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}],"provider_specific_fields":{"server_side_tool_invocations":[{"id":"s1","response":{}}]}},{"role":"tool","tool_call_id":"c1","content":"res"}]"#;
    let messages = parse_messages(source);
    let original = messages.clone();
    let first = converted_messages(&messages, Some("gemini-3-pro-preview"));
    let second = converted_messages(&messages, Some("gemini-3-pro-preview"));
    assert_eq!(messages, original);
    assert_eq!(first, second);
}

#[test]
fn builds_content_records_with_the_declared_roles() {
    let history = converted(
        r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a"}]"#,
        None,
    );
    assert_eq!(history.contents.len(), 2);
    assert_eq!(
        history.contents.first().map(|content| content.role),
        Some(Role::User)
    );
    assert_eq!(
        history.contents.last().map(|content| content.role),
        Some(Role::Model)
    );
    assert_eq!(
        history
            .contents
            .first()
            .and_then(|content| content.parts.first()),
        Some(&Object::from_iter([(
            "text".to_owned(),
            Json::Text("q".to_owned())
        )]))
    );
}

#[rstest]
#[case::plain_text_is_field_text(
    "{\"text\": \"hello\"}",
    r#"{"text":"hello","thoughtSignature":"sig"}"#
)]
#[case::thought_json_keeps_its_keys(
    "{\"text\": \"reason\", \"thought\": true}",
    r#"{"text":"reason","thought":true,"thoughtSignature":"sig"}"#
)]
#[case::empty_object("{}", r#"{"thoughtSignature":"sig"}"#)]
fn decodes_thinking_json_into_a_signed_part(#[case] thinking: &str, #[case] expected: &str) {
    let part = decode_thinking_part(thinking, "sig");
    let encoded = encode(&part);
    assert_eq!(encoded, parse_json(expected));
}

#[test]
fn refuses_thinking_json_past_the_byte_bound() {
    let filler = "a".repeat(MAX_THINKING_BYTES);
    let inside = format!("{{\"text\":\"{filler}\"}}");
    assert!(inside.len() > MAX_THINKING_BYTES);
    assert_eq!(
        decode_thinking_part(&inside, "sig").get("text"),
        Some(&Json::Text(inside.clone()))
    );

    let room = MAX_THINKING_BYTES.saturating_sub("{\"text\":\"\"}".len());
    let exact = format!("{{\"text\":\"{}\"}}", "a".repeat(room));
    assert_eq!(exact.len(), MAX_THINKING_BYTES);
    assert_eq!(
        decode_thinking_part(&exact, "sig").get("text"),
        Some(&Json::Text("a".repeat(room)))
    );

    let over = format!("{{\"text\":\"{}\"}}", "a".repeat(room.saturating_add(1)));
    assert_eq!(over.len(), MAX_THINKING_BYTES.saturating_add(1));
    assert_eq!(
        decode_thinking_part(&over, "sig").get("text"),
        Some(&Json::Text(over.clone()))
    );
}

#[test]
fn falls_back_to_the_signed_text_past_the_byte_bound() {
    let over = format!("{{\"text\":\"{}\"}}", "a".repeat(MAX_THINKING_BYTES));
    let part = decode_thinking_part(&over, "sig");
    assert_eq!(part.get("text"), Some(&Json::Text(over)));
    assert_eq!(
        part.get("thoughtSignature"),
        Some(&Json::Text("sig".to_owned()))
    );
}

#[rstest]
fn replays_the_thinking_part_corpus(#[files("fuzz/corpus/thinking_part/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(thinking) = from_utf8(&bytes) else {
        return;
    };
    drop(decode_thinking_part(thinking, "signature"));
    drop(decode_thinking_part(thinking, ""));
}
