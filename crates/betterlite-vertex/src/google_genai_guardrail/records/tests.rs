#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::google_genai_guardrail::callbacks::merged_request_data;
use crate::google_genai_guardrail::records::{
    Location, Segment, project_request, request_texts, response_texts, write_texts,
};
use crate::google_genai_guardrail::sse::{accumulated_stream_text, decode_google_guardrail_sse};
use crate::google_native_content::request::GenerateContentRequest;

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::root_camel_before_root_snake(
    r#"{"systemInstruction":{"parts":[{"text":"camel"}]},"system_instruction":{"parts":[{"text":"snake"}]},"contents":[{"parts":[{"text":"body"}]}]}"#,
    &["camel", "body"]
)]
#[case::root_snake_after_a_null_camel(
    r#"{"systemInstruction":null,"system_instruction":{"parts":[{"text":"snake"}]}}"#,
    &["snake"]
)]
#[case::empty_root_value_blocks_the_config(
    r#"{"systemInstruction":"","config":{"systemInstruction":{"parts":[{"text":"config"}]}}}"#,
    &[]
)]
#[case::config_camel_before_config_snake(
    r#"{"config":{"systemInstruction":{"parts":[{"text":"camel"}]},"system_instruction":{"parts":[{"text":"snake"}]}}}"#,
    &["camel"]
)]
#[case::config_snake(
    r#"{"config":{"system_instruction":{"parts":[{"text":"snake"}]}}}"#,
    &["snake"]
)]
#[case::config_snake_after_a_null_camel(
    r#"{"config":{"systemInstruction":null,"system_instruction":{"parts":[{"text":"snake"}]}}}"#,
    &["snake"]
)]
fn request_system_instruction_precedence(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(request_texts(&object(source)).1, expected.to_vec());
}

#[rstest]
#[case::single_content_object(r#"{"contents":{"parts":[{"text":"only"}]}}"#, &["only"])]
#[case::list_of_contents(
    r#"{"contents":[{"parts":[{"text":"a"}]},"ignored",{"parts":[{"text":"b"}]}]}"#,
    &["a", "b"]
)]
#[case::images_empty_and_non_strings_are_skipped(
    r#"{"contents":[{"parts":[{"text":"keep"},{"inlineData":{"mimeType":"image/png","data":"x"}},{"text":""},{"text":7}]}]}"#,
    &["keep"]
)]
#[case::no_contents(r#"{"model":"gemini"}"#, &[])]
fn contents_are_one_object_or_a_list(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(request_texts(&object(source)).1, expected.to_vec());
}

#[test]
fn request_text_locations_place_the_system_instruction_first() {
    let data = object(
        r#"{"model":"gemini","systemInstruction":{"parts":[{"text":"sys"},{"text":""}]},"contents":[{"parts":[{"text":"one"},{"inlineData":{}}]},{"parts":[{"text":"two"}]}]}"#,
    );
    assert_eq!(
        request_texts(&data).0,
        vec![
            vec![
                Segment::Key("systemInstruction"),
                Segment::Key("parts"),
                Segment::Index(0),
            ],
            vec![
                Segment::Key("contents"),
                Segment::Index(0),
                Segment::Key("parts"),
                Segment::Index(0),
            ],
            vec![
                Segment::Key("contents"),
                Segment::Index(1),
                Segment::Key("parts"),
                Segment::Index(0),
            ],
        ]
    );
}

#[test]
fn request_write_back_rewrites_only_located_text_fields() {
    let mut data = object(
        r#"{"model":"gemini","systemInstruction":{"role":"system","parts":[{"text":"sys"},{"inlineData":{"mimeType":"image/png"}}]},"contents":[{"role":"user","parts":[{"text":"one"},{"text":""}]},{"parts":[{"text":"two"}]}]}"#,
    );
    let locations = request_texts(&data).0;
    write_texts(
        &mut data,
        &locations,
        &["SYS".to_owned(), "ONE".to_owned(), "TWO".to_owned()],
    );
    assert_eq!(
        Json::Object(data),
        json(
            r#"{"model":"gemini","systemInstruction":{"role":"system","parts":[{"text":"SYS"},{"inlineData":{"mimeType":"image/png"}}]},"contents":[{"role":"user","parts":[{"text":"ONE"},{"text":""}]},{"parts":[{"text":"TWO"}]}]}"#
        )
    );
}

#[test]
fn response_write_back_visits_every_candidate() {
    let mut response = object(
        r#"{"candidates":[{"content":{"parts":[{"text":"a"},{"inlineData":{}},{"text":""}]},"finishReason":"STOP"},{"content":{"parts":[{"text":"b"}]}}]}"#,
    );
    let locations = response_texts(&response).0;
    let candidate = |index| -> Location {
        vec![
            Segment::Key("candidates"),
            Segment::Index(index),
            Segment::Key("content"),
            Segment::Key("parts"),
            Segment::Index(0),
        ]
    };
    assert_eq!(locations, vec![candidate(0), candidate(1)]);
    write_texts(&mut response, &locations, &["A".to_owned(), "B".to_owned()]);
    assert_eq!(
        Json::Object(response),
        json(
            r#"{"candidates":[{"content":{"parts":[{"text":"A"},{"inlineData":{}},{"text":""}]},"finishReason":"STOP"},{"content":{"parts":[{"text":"B"}]}}]}"#
        )
    );
}

#[rstest]
#[case::short(vec!["only".to_owned()])]
#[case::empty(Vec::new())]
#[case::long(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])]
fn mismatched_replacement_lengths_do_not_write(#[case] replacement: Vec<String>) {
    let mut response = object(
        r#"{"candidates":[{"content":{"parts":[{"text":"a"}]}},{"content":{"parts":[{"text":"b"}]}}]}"#,
    );
    let original = Json::Object(response.clone());
    let locations = response_texts(&response).0;
    write_texts(&mut response, &locations, &replacement);
    assert_eq!(Json::Object(response), original);
}

#[test]
fn supplied_context_keys_and_metadata_are_retained() {
    let request_data = object(r#"{"model":"gemini","response":null,"litellm_metadata":{}}"#);
    let user_metadata = object(r#"{"user_api_key_user_id":"u1"}"#);
    let merged = merged_request_data(
        Some(&request_data),
        &user_metadata,
        "response",
        json(r#"{"candidates":[]}"#),
    );
    assert_eq!(
        Json::Object(merged),
        json(r#"{"model":"gemini","response":null,"litellm_metadata":{}}"#)
    );
}

#[test]
fn absent_keys_take_the_current_object_and_metadata() {
    let user_metadata = object(r#"{"user_api_key_user_id":"u1"}"#);
    let merged = merged_request_data(None, &user_metadata, "responses_so_far", json("[]"));
    assert_eq!(
        Json::Object(merged),
        json(r#"{"responses_so_far":[],"litellm_metadata":{"user_api_key_user_id":"u1"}}"#)
    );
}

#[test]
fn empty_metadata_is_not_injected() {
    let merged = merged_request_data(None, &Object::new(), "response", json("{}"));
    assert_eq!(Json::Object(merged), json(r#"{"response":{}}"#));
}

#[test]
fn object_chunk_texts_come_before_raw_sse_texts() {
    let frames = vec![
        json(r#"{"candidates":[{"content":{"parts":[{"text":"obj1"}]}}]}"#),
        Json::Text(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"sse1\"}]}}]}\n\n"
                .to_owned(),
        ),
        json(r#"{"candidates":[{"content":{"parts":[{"text":"obj2"}]}}]}"#),
        Json::Text(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"sse2\"}]}}]}\n\n"
                .to_owned(),
        ),
    ];
    assert_eq!(accumulated_stream_text(&frames), "obj1obj2sse1sse2");
}

#[test]
fn a_frame_split_across_chunks_is_scanned_after_joining() {
    let frame = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}]}}]}\n\n";
    let split = frame.len().saturating_div(2);
    let (first, second) = frame.split_at(split);
    assert_eq!(
        decode_google_guardrail_sse(&[first.as_bytes(), second.as_bytes()]),
        vec!["hello".to_owned()]
    );
}

#[rstest]
#[case::done(b"data: [DONE]\n")]
#[case::empty_payload(b"data:\n")]
#[case::invalid_json(b"data: {oops}\n")]
#[case::non_object_payload(b"data: [1,2]\n")]
#[case::other_lines(b"event: ping\n: comment\ndata: text\n")]
fn ignored_sse_payloads_produce_no_text(#[case] chunk: &[u8]) {
    assert_eq!(decode_google_guardrail_sse(&[chunk]), Vec::<String>::new());
}

#[rstest]
#[case::no_space(b"data:{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"x\"}]}}]}\n")]
#[case::carriage_returns(
    b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"x\"}]}}]}\r\ndata: [DONE]\r\n"
)]
#[case::unit_separator_padding(
    b"data:\x1f{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"x\"}]}}]}\x1f\n"
)]
fn data_lines_are_scanned_without_a_space(#[case] chunk: &[u8]) {
    assert_eq!(decode_google_guardrail_sse(&[chunk]), vec!["x".to_owned()]);
}

#[test]
fn each_chunk_decodes_with_replacement_before_the_join() {
    let payload = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"é\"}]}}]}\r\n\r\n";
    let bytes = payload.as_bytes();
    let index = payload.find('é').unwrap();
    let (first, second) = bytes.split_at(index.saturating_add(1));
    assert_eq!(
        decode_google_guardrail_sse(&[first, second]),
        vec!["\u{fffd}\u{fffd}".to_owned()]
    );
}

#[rstest]
fn replays_the_google_guardrail_sse_corpus(
    #[files("fuzz/corpus/google_guardrail_sse/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_google_guardrail_sse(&corpus_chunks(&bytes)));
}

// splits a file the way the fuzz target does: a stride taken from the first byte varies the chunk boundaries
fn corpus_chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let Some((first, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let stride = usize::from(*first).rem_euclid(7).saturating_add(1);
    rest.chunks(stride).collect()
}

#[test]
fn the_projected_native_request_reads_like_the_wire_body() {
    let request = GenerateContentRequest {
        model: "gemini-2.5-flash".to_owned(),
        contents: json(r#"[{"parts":[{"text":"question"}]}]"#),
        tools: Json::Null,
        generation_config: Object::new(),
        system_instruction: json(r#"{"parts":[{"text":"instruction"}]}"#),
    };
    let projected = project_request(&request);
    assert_eq!(
        request_texts(&projected).1,
        vec!["instruction".to_owned(), "question".to_owned()]
    );
}

#[test]
fn a_null_instruction_is_not_projected() {
    let request = GenerateContentRequest {
        model: "gemini-2.5-flash".to_owned(),
        contents: json(r#"[{"parts":[{"text":"question"}]}]"#),
        tools: Json::Null,
        generation_config: Object::new(),
        system_instruction: Json::Null,
    };
    assert_eq!(
        Json::Object(project_request(&request)),
        json(r#"{"model":"gemini-2.5-flash","contents":[{"parts":[{"text":"question"}]}]}"#)
    );
}

#[test]
fn response_texts_read_every_candidate_part() {
    let response = object(
        r#"{"candidates":[{"content":{"parts":[{"text":"a"},{"inlineData":{}}]}},{"content":{"parts":[{"text":"b"},{"text":""}]}}]}"#,
    );
    assert_eq!(
        response_texts(&response).1,
        vec!["a".to_owned(), "b".to_owned()]
    );
}
