#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use uuid::Uuid;

use crate::gemini_response_parts::content::{
    ThinkingBlock, assistant_content, thinking_blocks, thought_signatures,
};
use crate::gemini_response_parts::media::{AudioPart, audio_part, image_parts};
use crate::gemini_response_parts::projection::{
    CallMode, LogprobsResult, Part, PartContext, PartsError, project_parts,
};
use crate::gemini_response_parts::tools::{FunctionChunk, SignatureFields, invocations, transform};

const NOW: i64 = 1_700_000_000;
const EXPIRES_AT: i64 = 1_700_086_400;
const HEX: &str = "0123456789abcdef0123456789abcdef";
const AUDIO_ID: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn parts(source: &str) -> Vec<Part> {
    serde_json::from_str(source).unwrap()
}

fn rendered<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn sequence(mut values: Vec<Uuid>) -> impl FnMut() -> Uuid {
    move || values.remove(0)
}

fn fixed_ids() -> impl FnMut() -> Uuid {
    || Uuid::parse_str(HEX).unwrap()
}

fn context(cumulative_tool_call_index: u32, call_mode: CallMode) -> PartContext {
    PartContext {
        cumulative_tool_call_index,
        call_mode,
        now: NOW,
    }
}

#[rstest]
#[case::plain_text(r#"[{"text":"hello"}]"#, Some("hello"), None)]
#[case::joins_parts(r#"[{"text":"one"},{"text":"two"}]"#, Some("onetwo"), None)]
#[case::thought_text(r#"[{"text":"plan","thought":true}]"#, None, Some("plan"))]
#[case::thought_false_is_content(r#"[{"text":"visible","thought":false}]"#, Some("visible"), None)]
#[case::empty_text_adds_nothing(r#"[{"text":""}]"#, None, None)]
#[case::empty_thought_text_adds_nothing(r#"[{"text":"","thought":true}]"#, None, None)]
#[case::audio_data_uri_is_not_text(r#"[{"text":"data:audio/wav;base64,QUJD"}]"#, None, None)]
#[case::audiobook_data_uri_is_text(
    r#"[{"text":"data:audiobook;base64,QUJD"}]"#,
    Some("data:audiobook;base64,QUJD"),
    None
)]
#[case::invalid_audio_data_uri_is_text(
    r#"[{"text":"data:audio/wav;base64,!!!"}]"#,
    Some("data:audio/wav;base64,!!!"),
    None
)]
#[case::nonmedia_inline_data_becomes_a_data_uri(
    r#"[{"inlineData":{"mimeType":"application/pdf","data":"cGRm"}}]"#,
    Some("data:application/pdf;base64,cGRm"),
    None
)]
#[case::inline_data_without_mime_type_is_text(
    r#"[{"inlineData":{"data":"YWJj"}}]"#,
    Some("data:;base64,YWJj"),
    None
)]
#[case::image_inline_data_is_not_text(
    r#"[{"inlineData":{"mimeType":"image/png","data":"aW1n"}}]"#,
    None,
    None
)]
#[case::audio_inline_data_is_not_text(
    r#"[{"inlineData":{"mimeType":"audio/mp3","data":"YXVk"}}]"#,
    None,
    None
)]
#[case::text_wins_over_inline_data(
    r#"[{"text":"caption","inlineData":{"mimeType":"application/pdf","data":"cGRm"}}]"#,
    Some("caption"),
    None
)]
#[case::thought_inline_data_is_reasoning(
    r#"[{"inlineData":{"mimeType":"application/pdf","data":"cGRm"},"thought":true}]"#,
    None,
    Some("data:application/pdf;base64,cGRm")
)]
fn assistant_content_matches_litellm(
    #[case] source: &str,
    #[case] content: Option<&str>,
    #[case] reasoning: Option<&str>,
) {
    let (actual_content, actual_reasoning) = assistant_content(&parts(source));
    assert_eq!(actual_content.as_deref(), content);
    assert_eq!(actual_reasoning.as_deref(), reasoning);
}

#[rstest]
#[case::thought_with_signature(
    r#"[{"text":"plan","thought":true,"thoughtSignature":"sig"}]"#,
    r#"[{"type":"thinking","thinking":"plan","signature":"sig"}]"#
)]
#[case::thought_without_signature(
    r#"[{"text":"plan","thought":true}]"#,
    r#"[{"type":"thinking","thinking":"plan"}]"#
)]
#[case::thought_without_text(r#"[{"thought":true}]"#, r#"[{"type":"thinking","thinking":""}]"#)]
#[case::empty_signature_is_kept(
    r#"[{"text":"plan","thought":true,"thoughtSignature":""}]"#,
    r#"[{"type":"thinking","thinking":"plan","signature":""}]"#
)]
#[case::nonthought_parts_have_no_block(
    r#"[{"text":"visible","thoughtSignature":"sig"},{"text":"thoughtless"}]"#,
    "[]"
)]
fn thinking_blocks_match_litellm(#[case] source: &str, #[case] block: &str) {
    assert_eq!(rendered(&thinking_blocks(&parts(source))), expected(block));
}

#[rstest]
#[case::no_signatures(r#"[{"text":"plain"}]"#, "null")]
#[case::thought_signature(
    r#"[{"text":"plan","thought":true,"thoughtSignature":"one"}]"#,
    r#"["one"]"#
)]
#[case::every_part_counts(
    r#"[{"text":"a","thoughtSignature":"one"},{"functionCall":{"name":"f","args":{}},"thoughtSignature":"two"},{"toolResponse":{"id":"r"},"thoughtSignature":""}]"#,
    r#"["one","two",""]"#
)]
fn thought_signatures_collect_every_part(#[case] source: &str, #[case] signatures: &str) {
    assert_eq!(
        rendered(&thought_signatures(&parts(source))),
        expected(signatures)
    );
}

#[test]
fn images_use_zero_index_and_auto_detail() {
    let parts = parts(
        r#"[{"inlineData":{"mimeType":"image/png","data":"aW1n"}},
            {"text":"caption"},
            {"inlineData":{"mimeType":"audio/mp3","data":"YXVk"}},
            {"inlineData":{"mimeType":"image/jpeg","data":"anBn"},"text":"caption"}]"#,
    );
    assert_eq!(
        rendered(&image_parts(&parts)),
        expected(
            r#"[{"image_url":{"url":"data:image/png;base64,aW1n","detail":"auto"},"index":0,"type":"image_url"},
                {"image_url":{"url":"data:image/jpeg;base64,anBn","detail":"auto"},"index":0,"type":"image_url"}]"#
        )
    );
}

#[test]
fn no_inline_data_yields_no_images() {
    assert_eq!(
        rendered(&image_parts(&parts(r#"[{"text":"plain"}]"#))),
        expected("[]")
    );
}

#[rstest]
#[case::valid_text_uri(
    r#"[{"text":"data:audio/wav;base64,QUJD"}]"#,
    Some(("QUJD", EXPIRES_AT))
)]
#[case::empty_payload(r#"[{"text":"data:audio/wav;base64,"}]"#, Some(("", EXPIRES_AT)))]
#[case::valid_inline_audio(
    r#"[{"inlineData":{"mimeType":"audio/mp3","data":"YXVk"}}]"#,
    Some(("YXVk", EXPIRES_AT))
)]
#[case::pending_padding_is_not_canonical(r#"[{"text":"data:audio/wav;base64,QQ"}]"#, None)]
#[case::trailing_bits_are_not_canonical(r#"[{"text":"data:audio/wav;base64,QR=="}]"#, None)]
#[case::invalid_payload(r#"[{"text":"data:audio/wav;base64,!!!"}]"#, None)]
#[case::missing_base64_marker(r#"[{"text":"data:audio/wav,QUJD"}]"#, None)]
#[case::a_second_marker_fails_the_split(
    r#"[{"text":"data:audio/wav;base64,,;base64,QUJD"}]"#,
    None
)]
#[case::missing_comma(r#"[{"text":"data:audio/wav;base64QUJD"}]"#, None)]
#[case::audiobook_is_not_audio(r#"[{"text":"data:audiobook;base64,QUJD"}]"#, None)]
#[case::non_audio_text_is_skipped(
    r#"[{"text":"plain"},{"inlineData":{"mimeType":"audio/mp3","data":"YXVk"}}]"#,
    Some(("YXVk", EXPIRES_AT))
)]
#[case::text_branch_hides_inline_audio(
    r#"[{"text":"plain","inlineData":{"mimeType":"audio/mp3","data":"YXVk"}}]"#,
    None
)]
#[case::image_inline_data_is_not_audio(
    r#"[{"inlineData":{"mimeType":"image/png","data":"aW1n"}}]"#,
    None
)]
#[case::no_audio_at_all(r#"[{"text":"plain"}]"#, None)]
fn audio_part_picks_the_first_audio(
    #[case] source: &str,
    #[case] expected_audio: Option<(&str, i64)>,
) {
    let mut ids = fixed_ids();
    let audio = audio_part(&parts(source), NOW, &mut ids).unwrap();
    assert_eq!(
        audio.map(|clip: AudioPart| (clip.data, clip.expires_at)),
        expected_audio.map(|(data, expires_at)| (data.to_owned(), expires_at))
    );
}

#[test]
fn audio_uses_the_injected_clock_and_identity() {
    let mut ids = fixed_ids();
    let audio = audio_part(
        &parts(r#"[{"text":"data:audio/wav;base64,QUJD"}]"#),
        NOW,
        &mut ids,
    )
    .unwrap()
    .unwrap();
    assert_eq!(audio.id, AUDIO_ID);
    assert_eq!(audio.expires_at, EXPIRES_AT);
    assert_eq!(audio.transcript, "");
}

#[test]
fn audio_expiry_overflow_is_an_error() {
    let mut ids = fixed_ids();
    let result = audio_part(
        &parts(r#"[{"text":"data:audio/wav;base64,QUJD"}]"#),
        i64::MAX,
        &mut ids,
    );
    assert!(matches!(result, Err(PartsError::Expiry { now: i64::MAX })));
}

#[test]
fn an_audio_uri_ends_at_the_next_data_segment() {
    let mut ids = fixed_ids();
    let result = audio_part(
        &parts(r#"[{"text":"data:audio/xdata:y;base64,QUJD"}]"#),
        NOW,
        &mut ids,
    );
    assert!(result.unwrap().is_none());
}

#[test]
fn a_second_data_segment_after_the_marker_fails_the_split() {
    let mut ids = fixed_ids();
    let result = audio_part(
        &parts(r#"[{"text":"data:audio/wavdata:;base64,QUJD"}]"#),
        NOW,
        &mut ids,
    );
    assert!(result.unwrap().is_none());
}

#[rstest]
#[case::matched_call_and_response(
    r#"[{"toolCall":{"toolType":"search","id":"one","args":{"q":"x"}}},
        {"toolResponse":{"toolType":"search","id":"one","response":{"ok":true}}}]"#,
    r#"[{"tool_type":"search","id":"one","args":{"q":"x"},"response":{"ok":true}}]"#
)]
#[case::matched_response_without_a_response_field_is_null(
    r#"[{"toolCall":{"id":"one"}},{"toolResponse":{"id":"one"}}]"#,
    r#"[{"tool_type":null,"id":"one","args":null,"response":null}]"#
)]
#[case::unmatched_call_has_no_response_key(
    r#"[{"toolCall":{"toolType":"maps","id":"missing"}}]"#,
    r#"[{"tool_type":"maps","id":"missing","args":null}]"#
)]
#[case::orphan_response_is_kept(
    r#"[{"toolResponse":{"toolType":"maps","id":"orphan","response":"done"}}]"#,
    r#"[{"id":"orphan","tool_type":"maps","response":"done"}]"#
)]
#[case::call_and_response_signatures_are_separate(
    r#"[{"toolCall":{"id":"one","args":{}},"thoughtSignature":"call-sig"},
        {"toolResponse":{"id":"one","response":{}},"thoughtSignature":"response-sig"}]"#,
    r#"[{"tool_type":null,"id":"one","args":{},"thought_signature":"call-sig","response":{},"response_thought_signature":"response-sig"}]"#
)]
#[case::orphan_response_keeps_both_signature_fields(
    r#"[{"toolResponse":{"id":"orphan","response":null},"thoughtSignature":"sig"}]"#,
    r#"[{"id":"orphan","tool_type":null,"response":null,"thought_signature":"sig","response_thought_signature":"sig"}]"#
)]
#[case::calls_come_before_orphan_responses(
    r#"[{"toolResponse":{"id":"orphan","response":1}},
        {"toolCall":{"id":"call","args":{}}}]"#,
    r#"[{"tool_type":null,"id":"call","args":{}},{"id":"orphan","tool_type":null,"response":1}]"#
)]
#[case::parts_without_server_tools(r#"[{"text":"plain"}]"#, "null")]
fn server_tools_pair_calls_and_responses(#[case] source: &str, #[case] invocations_json: &str) {
    assert_eq!(
        rendered(&invocations(&parts(source))),
        expected(invocations_json)
    );
}

#[test]
fn server_tools_repeated_ids_overwrite_in_place() {
    let parts = parts(
        r#"[{"toolCall":{"id":"one","args":{"first":1}}},
            {"toolCall":{"id":"two","args":{"second":2}}},
            {"toolCall":{"id":"one","args":{"last":3}}},
            {"toolResponse":{"id":"two","response":"early"}},
            {"toolResponse":{"id":"two","response":"late"}}]"#,
    );
    assert_eq!(
        rendered(&invocations(&parts)),
        expected(
            r#"[{"tool_type":null,"id":"one","args":{"last":3}},
                {"tool_type":null,"id":"two","args":{"second":2},"response":"late"}]"#
        )
    );
}

#[test]
fn server_tools_without_an_id_pair_on_the_empty_key() {
    let parts = parts(
        r#"[{"toolCall":{"args":{"first":1}}},
            {"toolCall":{"args":{"last":2}}},
            {"toolResponse":{"response":"paired"}}]"#,
    );
    assert_eq!(
        rendered(&invocations(&parts)),
        expected(r#"[{"tool_type":null,"id":null,"args":{"last":2},"response":"paired"}]"#)
    );
}

#[test]
fn a_tool_call_wins_over_a_tool_response_in_one_part() {
    let parts = parts(r#"[{"toolCall":{"id":"call"},"toolResponse":{"id":"response"}}]"#);
    assert_eq!(
        rendered(&invocations(&parts)),
        expected(r#"[{"tool_type":null,"id":"call","args":null}]"#)
    );
}

#[rstest]
#[case::native_ids_pass_through(
    r#"[{"functionCall":{"name":"f","args":{},"id":"native"}}]"#,
    r#"[{"id":"native","type":"function","function":{"name":"f","arguments":"{}"},"index":4}]"#
)]
#[case::synthetic_ids_come_from_the_source(
    r#"[{"functionCall":{"name":"f","args":{"a":1}}}]"#,
    r#"[{"id":"call_0123456789abcdef0123456789ab","type":"function","function":{"name":"f","arguments":"{\"a\":1}"},"index":4}]"#
)]
#[case::signatures_encode_into_ids(
    r#"[{"functionCall":{"name":"f","args":{},"id":"native"},"thoughtSignature":"sig"}]"#,
    r#"[{"id":"native__thought__sig","type":"function","function":{"name":"f","arguments":"{}"},"index":4,"provider_specific_fields":{"thought_signature":"sig"}}]"#
)]
#[case::synthetic_ids_take_a_signature_too(
    r#"[{"functionCall":{"name":"f","args":{}},"thoughtSignature":"sig"}]"#,
    r#"[{"id":"call_0123456789abcdef0123456789ab__thought__sig","type":"function","function":{"name":"f","arguments":"{}"},"index":4,"provider_specific_fields":{"thought_signature":"sig"}}]"#
)]
#[case::empty_signatures_stay_as_given(
    r#"[{"functionCall":{"name":"f","args":{},"id":"native"},"thoughtSignature":""}]"#,
    r#"[{"id":"native","type":"function","function":{"name":"f","arguments":"{}"},"index":4,"provider_specific_fields":{"thought_signature":""}}]"#
)]
fn tool_calls_use_native_and_synthetic_ids(#[case] source: &str, #[case] tools_json: &str) {
    let mut ids = fixed_ids();
    let (function, tools, next) = transform(&parts(source), 4, CallMode::Tools, &mut ids).unwrap();
    assert!(function.is_none());
    assert_eq!(next, 5);
    assert_eq!(rendered(&tools), expected(tools_json));
}

#[test]
fn tool_mode_functions_carry_no_provider_fields() {
    let mut ids = fixed_ids();
    let (_, tools, _) = transform(
        &parts(r#"[{"functionCall":{"name":"f","args":{}},"thoughtSignature":"sig"}]"#),
        0,
        CallMode::Tools,
        &mut ids,
    )
    .unwrap();
    let tool = tools.unwrap().remove(0);
    assert_eq!(
        rendered(&tool.function),
        expected(r#"{"name":"f","arguments":"{}"}"#)
    );
    assert_eq!(
        rendered(&tool.provider_specific_fields),
        expected(r#"{"thought_signature":"sig"}"#)
    );
}

#[rstest]
#[case::keeps_the_last_call(
    r#"[{"functionCall":{"name":"first","args":{"a":1}}},{"functionCall":{"name":"last","args":{"b":2}}}]"#,
    r#"{"name":"last","arguments":"{\"b\":2}"}"#
)]
#[case::carries_a_signature(
    r#"[{"functionCall":{"name":"f","args":{}},"thoughtSignature":"sig"}]"#,
    r#"{"name":"f","arguments":"{}","provider_specific_fields":{"thought_signature":"sig"}}"#
)]
#[case::empty_signature_stays_as_given(
    r#"[{"functionCall":{"name":"f","args":{}},"thoughtSignature":""}]"#,
    r#"{"name":"f","arguments":"{}","provider_specific_fields":{"thought_signature":""}}"#
)]
#[case::ignores_other_parts(r#"[{"text":"plain"}]"#, "null")]
fn legacy_function_keeps_the_last_call(#[case] source: &str, #[case] function_json: &str) {
    let mut ids = fixed_ids();
    let (function, tools, next) =
        transform(&parts(source), 2, CallMode::LegacyFunction, &mut ids).unwrap();
    assert!(tools.is_none());
    assert_eq!(rendered(&function), expected(function_json));
    let calls = parts(source)
        .iter()
        .filter(|part| part.function_call.is_some())
        .count();
    assert_eq!(next, u32::try_from(calls).unwrap().checked_add(2).unwrap());
}

#[test]
fn tool_calls_are_none_without_function_calls() {
    let mut ids = fixed_ids();
    let (function, tools, next) = transform(
        &parts(r#"[{"text":"plain"}]"#),
        7,
        CallMode::Tools,
        &mut ids,
    )
    .unwrap();
    assert!(function.is_none());
    assert!(tools.is_none());
    assert_eq!(next, 7);
}

#[test]
fn arguments_keep_unicode_and_input_order() {
    let mut ids = fixed_ids();
    let (_, tools, _) = transform(
        &parts(r#"[{"functionCall":{"name":"f","args":{"b":"lλ","a":1}}}]"#),
        0,
        CallMode::Tools,
        &mut ids,
    )
    .unwrap();
    let arguments = tools.unwrap().remove(0).function.arguments;
    assert_eq!(arguments, "{\"b\":\"l\u{03bb}\",\"a\":1}");
}

#[rstest]
#[case::absent_args(r#"[{"functionCall":{"name":"f"}}]"#, "{}")]
#[case::null_args(r#"[{"functionCall":{"name":"f","args":null}}]"#, "{}")]
#[case::empty_args(r#"[{"functionCall":{"name":"f","args":{}}}]"#, "{}")]
fn absent_function_arguments_become_an_empty_object(#[case] source: &str, #[case] arguments: &str) {
    let mut ids = fixed_ids();
    let (function, _, _) =
        transform(&parts(source), 0, CallMode::LegacyFunction, &mut ids).unwrap();
    assert_eq!(function.unwrap().arguments, arguments);
}

#[test]
fn a_function_call_without_a_name_is_rejected() {
    let decoded = serde_json::from_str::<Vec<Part>>(r#"[{"functionCall":{"args":{}}}]"#);
    assert!(decoded.unwrap_err().to_string().contains("name"));
}

#[test]
fn tool_index_overflow_is_an_error() {
    let mut ids = fixed_ids();
    let result = transform(
        &parts(r#"[{"functionCall":{"name":"f","args":{}}}]"#),
        u32::MAX,
        CallMode::Tools,
        &mut ids,
    );
    assert!(matches!(result, Err(PartsError::ToolIndex)));
}

#[test]
fn project_parts_reads_the_audio_id_before_the_tool_id() {
    let second = Uuid::parse_str("fedcba9876543210fedcba9876543210").unwrap();
    let mut ids = sequence(vec![Uuid::parse_str(HEX).unwrap(), second]);
    let projection = project_parts(
        &parts(
            r#"[{"text":"data:audio/wav;base64,QUJD"},{"functionCall":{"name":"f","args":{}}}]"#,
        ),
        None,
        context(0, CallMode::Tools),
        &mut ids,
    )
    .unwrap();
    assert_eq!(projection.audio.unwrap().id, AUDIO_ID);
    assert_eq!(
        projection.tool_calls.unwrap()[0].id,
        "call_fedcba9876543210fedcba987654"
    );
}

#[test]
fn project_parts_exposes_every_response_field() {
    let mut ids = fixed_ids();
    let result: LogprobsResult = serde_json::from_str(
        r#"{"chosenCandidates":[{"token":"a","tokenId":1,"logProbability":-0.5}]}"#,
    )
    .unwrap();
    let projection = project_parts(
        &parts(
            r#"[{"text":"think","thought":true,"thoughtSignature":"sig-one"},
                {"text":"visible"},
                {"inlineData":{"mimeType":"image/png","data":"aW1n"}},
                {"text":"data:audio/wav;base64,QUJD"},
                {"functionCall":{"name":"f","args":{"x":1},"id":"native"},"thoughtSignature":"sig-two"},
                {"toolCall":{"id":"tool","args":{}}},
                {"toolResponse":{"id":"tool","response":"done"}}]"#,
        ),
        Some(&result),
        context(6, CallMode::Tools),
        &mut ids,
    )
    .unwrap();
    assert_eq!(
        rendered(&projection),
        expected(
            r#"{"content":"visible","reasoning_content":"think",
                "thinking_blocks":[{"type":"thinking","thinking":"think","signature":"sig-one"}],
                "thought_signatures":["sig-one","sig-two"],
                "server_side_tool_invocations":[{"tool_type":null,"id":"tool","args":{},"response":"done"}],
                "images":[{"image_url":{"url":"data:image/png;base64,aW1n","detail":"auto"},"index":0,"type":"image_url"}],
                "audio":{"id":"01234567-89ab-cdef-0123-456789abcdef","data":"QUJD","expires_at":1700086400,"transcript":""},
                "function":null,
                "tool_calls":[{"id":"native__thought__sig-two","type":"function","function":{"name":"f","arguments":"{\"x\":1}"},"index":6,"provider_specific_fields":{"thought_signature":"sig-two"}}],
                "next_tool_call_index":7,
                "logprobs":{"content":[{"token":"a","logprob":-0.5,"top_logprobs":[]}]}}"#
        )
    );
}

#[test]
fn generated_tool_ids_have_the_synthetic_shape() {
    let mut ids = Uuid::new_v4;
    let (_, tools, _) = transform(
        &parts(r#"[{"functionCall":{"name":"f","args":{}}}]"#),
        0,
        CallMode::Tools,
        &mut ids,
    )
    .unwrap();
    let id = tools.unwrap().remove(0).id;
    assert!(id.starts_with("call_"));
    assert_eq!(id.len(), "call_".len() + 28);
    assert!(
        id.chars()
            .skip("call_".len())
            .all(|character| character.is_ascii_hexdigit())
    );
}

fn logprobs_of(source: Option<&str>) -> Json {
    let mut ids = fixed_ids();
    let result = source.map(|text| serde_json::from_str::<LogprobsResult>(text).unwrap());
    let projection =
        project_parts(&[], result.as_ref(), context(0, CallMode::Tools), &mut ids).unwrap();
    rendered(&projection.logprobs)
}

#[test]
fn logprobs_without_a_result_are_none() {
    assert_eq!(logprobs_of(None), expected("null"));
}

#[test]
fn logprobs_without_chosen_candidates_are_none() {
    assert_eq!(
        logprobs_of(Some(r#"{"topCandidates":[]}"#)),
        expected("null")
    );
}

#[test]
fn empty_chosen_candidates_give_empty_content() {
    assert_eq!(
        logprobs_of(Some(r#"{"chosenCandidates":[]}"#)),
        expected(r#"{"content":[]}"#)
    );
}

#[test]
fn logprobs_keep_sparse_top_candidates() {
    assert_eq!(
        logprobs_of(Some(
            r#"{"chosenCandidates":[{"token":"a","tokenId":1,"logProbability":-0.5},
                                   {"token":"b","tokenId":2,"logProbability":-0.25}],
               "topCandidates":[{"candidates":[{"token":"a","tokenId":1,"logProbability":-0.1},
                                               {"token":"c","tokenId":3,"logProbability":-0.9}]}]}"#
        )),
        expected(
            r#"{"content":[
                {"token":"a","logprob":-0.5,"top_logprobs":[{"token":"a","logprob":-0.1},{"token":"c","logprob":-0.9}]},
                {"token":"b","logprob":-0.25,"top_logprobs":[]}]}"#
        )
    );
}

#[test]
fn logprobs_ignore_tops_past_the_chosen_list() {
    assert_eq!(
        logprobs_of(Some(
            r#"{"chosenCandidates":[{"token":"a","tokenId":1,"logProbability":-0.5}],
               "topCandidates":[{"candidates":[]},{"candidates":[{"token":"z","tokenId":9,"logProbability":-1.0}]}]}"#
        )),
        expected(r#"{"content":[{"token":"a","logprob":-0.5,"top_logprobs":[]}]}"#)
    );
}

#[test]
fn a_logprob_candidate_needs_a_token() {
    let decoded = serde_json::from_str::<LogprobsResult>(r#"{"chosenCandidates":[{"tokenId":1}]}"#);
    assert!(decoded.unwrap_err().to_string().contains("token"));
}

#[test]
fn a_top_candidate_needs_its_candidates() {
    let decoded = serde_json::from_str::<LogprobsResult>(r#"{"topCandidates":[{}]}"#);
    assert!(decoded.unwrap_err().to_string().contains("candidates"));
}

#[test]
fn chunk_types_expose_the_signature_fields() {
    let chunk = FunctionChunk {
        name: "f".to_owned(),
        arguments: "{}".to_owned(),
        provider_specific_fields: Some(SignatureFields {
            thought_signature: "sig".to_owned(),
        }),
    };
    assert_eq!(
        rendered(&chunk),
        expected(
            r#"{"name":"f","arguments":"{}","provider_specific_fields":{"thought_signature":"sig"}}"#
        )
    );
    let block = ThinkingBlock::Thinking {
        thinking: "plan".to_owned(),
        signature: None,
    };
    assert_eq!(
        rendered(&block),
        expected(r#"{"type":"thinking","thinking":"plan"}"#)
    );
}
