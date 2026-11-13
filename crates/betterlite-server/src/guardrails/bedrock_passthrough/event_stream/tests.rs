#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::path::Path;
use std::sync::Mutex;

use aws_smithy_eventstream::frame::{read_message_from, write_message_to};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::frames::{Frame, ParsedStream, parse_event_stream, rebuild_event_stream};
use super::groups::{GroupKey, group_texts, redistribute};
use super::hook::{
    EventStreamError, PostCallSuccessHook, de_anonymize_event_stream, value_type_name,
};

fn raw_frame(headers: &[(&str, &str)], payload: &[u8]) -> Vec<u8> {
    let mut message = Message::new(payload.to_vec());
    for (name, value) in headers {
        let header = Header::new(
            (*name).to_owned(),
            HeaderValue::String(StrBytes::from((*value).to_owned())),
        );
        message = message.add_header(header);
    }
    let mut buffer = Vec::new();
    write_message_to(&message, &mut buffer).expect("writes a frame");
    buffer
}

fn delta_frame(delta: &str) -> Vec<u8> {
    let payload = format!("{{\"delta\": {delta}}}");
    raw_frame(
        &[
            (":event-type", "contentBlockDelta"),
            (":message-type", "event"),
        ],
        payload.as_bytes(),
    )
}

fn event_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
    raw_frame(
        &[(":event-type", event_type), (":message-type", "event")],
        payload,
    )
}

fn length_at(raw: &[u8], start: usize) -> usize {
    let bytes = raw.get(start..start.saturating_add(4)).expect("prelude");
    usize::try_from(u32::from_be_bytes(bytes.try_into().expect("four bytes"))).expect("length fits")
}

fn frame_parts(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let headers_length = length_at(raw, 4);
    let headers = raw
        .get(12..12_usize.saturating_add(headers_length))
        .expect("headers bounds");
    let payload = raw
        .get(12_usize.saturating_add(headers_length)..length_at(raw, 0).saturating_sub(4))
        .expect("payload bounds");
    (headers.to_vec(), payload.to_vec())
}

fn frame_texts(frames: &[Frame]) -> Vec<Vec<String>> {
    frames
        .iter()
        .map(|frame| frame.texts.iter().map(|(_, text)| text.clone()).collect())
        .collect()
}

#[derive(Debug, thiserror::Error)]
#[error("scripted hook failure")]
struct HookFailure;

enum Mode {
    Replace,
    PassThrough,
    NotAnObject,
    DropFirstBlock,
    NonTextBlock,
    Fail,
}

struct Hook {
    mode: Mode,
    replace: BTreeMap<String, String>,
    seen: Mutex<Option<GuardrailValue>>,
}

impl Hook {
    fn replacing(pairs: &[(&str, &str)]) -> Self {
        Self {
            mode: Mode::Replace,
            replace: pairs
                .iter()
                .map(|(text, replacement)| ((*text).to_owned(), (*replacement).to_owned()))
                .collect(),
            seen: Mutex::new(None),
        }
    }

    fn with_mode(mode: Mode) -> Self {
        Self {
            mode,
            replace: BTreeMap::new(),
            seen: Mutex::new(None),
        }
    }

    fn seen(&self) -> GuardrailValue {
        self.seen
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("the hook ran")
    }
}

impl PostCallSuccessHook for Hook {
    type Error = HookFailure;

    fn post_call_success_hook(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        _user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send {
        *self.seen.lock().expect("not poisoned") = Some(response.clone());
        let result = match self.mode {
            Mode::Replace => Ok(replace_all(&response, &self.replace)),
            Mode::PassThrough => Ok(response),
            Mode::NotAnObject => Ok(GuardrailValue::Text("not an object".to_owned())),
            Mode::DropFirstBlock => Ok(drop_first_block(&response)),
            Mode::NonTextBlock => Ok(with_blocks(vec![GuardrailValue::Object(BTreeMap::from([
                ("text".to_owned(), GuardrailValue::SignedInteger(7)),
            ]))])),
            Mode::Fail => Err(HookFailure),
        };
        ready(result)
    }
}

fn blocks_of(response: &GuardrailValue) -> Vec<GuardrailValue> {
    let GuardrailValue::Object(fields) = response else {
        return Vec::new();
    };
    let Some(GuardrailValue::Object(output)) = fields.get("output") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Object(message)) = output.get("message") else {
        return Vec::new();
    };
    match message.get("content") {
        Some(GuardrailValue::Array(blocks)) => blocks.clone(),
        _ => Vec::new(),
    }
}

fn replace_all(response: &GuardrailValue, replace: &BTreeMap<String, String>) -> GuardrailValue {
    let mut blocks = blocks_of(response);
    for block in &mut blocks {
        let GuardrailValue::Object(fields) = block else {
            continue;
        };
        if let Some(GuardrailValue::Text(text)) = fields.get_mut("text")
            && let Some(replacement) = replace.get(text.as_str())
        {
            text.clone_from(replacement);
        }
    }
    with_blocks(blocks)
}

fn drop_first_block(response: &GuardrailValue) -> GuardrailValue {
    let mut blocks = blocks_of(response);
    blocks.remove(0);
    with_blocks(blocks)
}

fn with_blocks(blocks: Vec<GuardrailValue>) -> GuardrailValue {
    let mut message = BTreeMap::new();
    message.insert(
        "role".to_owned(),
        GuardrailValue::Text("assistant".to_owned()),
    );
    message.insert("content".to_owned(), GuardrailValue::Array(blocks));
    let mut output = BTreeMap::new();
    output.insert("message".to_owned(), GuardrailValue::Object(message));
    let mut response = BTreeMap::new();
    response.insert("output".to_owned(), GuardrailValue::Object(output));
    response.insert(
        "stopReason".to_owned(),
        GuardrailValue::Text("end_turn".to_owned()),
    );
    GuardrailValue::Object(response)
}

async fn anonymize_with(body: &[u8], hook: &Hook) -> Vec<u8> {
    de_anonymize_event_stream(body, hook, &BTreeMap::new(), &BTreeMap::new())
        .await
        .expect("the hook answers")
}

#[tokio::test]
async fn a_text_delta_is_rewritten_with_correct_proportional_slices() {
    let body = [
        delta_frame(r#"{"text": "abc"}"#),
        delta_frame(r#"{"text": "de"}"#),
    ]
    .concat();
    let hook = Hook::replacing(&[("abcde", "ABCDEF")]);
    let output = anonymize_with(&body, &hook).await;
    let parsed = parse_event_stream(&output);
    assert_eq!(parsed.frames.len(), 2);
    assert_eq!(frame_texts(&parsed.frames), vec![vec!["ABCD"], vec!["EF"]]);
    assert!(parsed.trailing.is_empty());
}

#[tokio::test]
async fn split_fragments_are_joined_before_the_hook_sees_them() {
    let body = [
        delta_frame(r#"{"text": "abc"}"#),
        delta_frame(r#"{"text": "de"}"#),
    ]
    .concat();
    let hook = Hook::with_mode(Mode::PassThrough);
    anonymize_with(&body, &hook).await;
    let seen = hook.seen();
    assert_eq!(
        blocks_of(&seen),
        vec![GuardrailValue::Object(BTreeMap::from([(
            "text".to_owned(),
            GuardrailValue::Text("abcde".to_owned()),
        )]))]
    );
}

#[tokio::test]
async fn the_synthetic_response_carries_one_block_per_active_group_in_order() {
    let body = [
        delta_frame(r#"{"text": "first"}"#),
        delta_frame(r#"{"toolUse": {"input": "second"}}"#),
        delta_frame(r#"{"reasoningContent": {"text": "third"}}"#),
    ]
    .concat();
    let hook = Hook::with_mode(Mode::PassThrough);
    anonymize_with(&body, &hook).await;
    let blocks = blocks_of(&hook.seen());
    let texts: Vec<GuardrailValue> = blocks
        .iter()
        .filter_map(|block| {
            let GuardrailValue::Object(fields) = block else {
                return None;
            };
            fields.get("text").cloned()
        })
        .collect();
    assert_eq!(
        texts,
        vec![
            GuardrailValue::Text("first".to_owned()),
            GuardrailValue::Text("second".to_owned()),
            GuardrailValue::Text("third".to_owned()),
        ]
    );
}

#[test]
fn delta_cells_cover_text_tool_reasoning_and_citations_in_order() {
    let body = delta_frame(
        r#"{"text": "a", "toolUse": {"input": "b"}, "reasoningContent": {"text": "c", "signature": "skip"}, "citationsContent": {"content": [{"text": "d"}, "skip", {"text": "e"}]}}"#,
    );
    let parsed = parse_event_stream(&body);
    let cells = &parsed.frames[0].texts;
    assert_eq!(
        cells,
        &vec![
            (GroupKey::Text, "a".to_owned()),
            (GroupKey::Tool, "b".to_owned()),
            (GroupKey::Reasoning, "c".to_owned()),
            (GroupKey::Citation(0), "d".to_owned()),
            (GroupKey::Citation(2), "e".to_owned()),
        ]
    );
}

#[test]
fn an_empty_delta_text_is_a_cell_and_the_group_stays_inactive() {
    let body = delta_frame(r#"{"text": ""}"#);
    let parsed = parse_event_stream(&body);
    assert_eq!(
        parsed.frames[0].texts,
        vec![(GroupKey::Text, String::new())]
    );
    let groups = group_texts(&parsed.frames);
    assert_eq!(groups.groups.len(), 1);
    assert!(
        redistribute(&groups, &["REPLACED".to_owned()])
            .expect("shares")
            .is_empty()
    );
}

#[tokio::test]
async fn a_stream_without_active_groups_is_returned_unchanged() {
    let body = [
        event_frame("messageStart", b"{\"role\": \"assistant\"}"),
        delta_frame(r#"{"text": ""}"#),
    ]
    .concat();
    let hook = Hook::replacing(&[("", "ignored")]);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output, body);
}

#[tokio::test]
async fn a_frame_that_is_not_a_content_block_delta_is_copied() {
    let start = event_frame("contentBlockStart", br#"{"delta": {"text": "hello"}}"#);
    let body = [start.clone(), delta_frame(r#"{"text": "hello"}"#)].concat();
    let parsed = parse_event_stream(&body);
    assert!(parsed.frames[0].texts.is_empty());
    let hook = Hook::replacing(&[("hello", "HELLO")]);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output.get(0..start.len()), Some(start.as_slice()));
}

#[tokio::test]
async fn a_frame_with_a_broken_crc_is_forwarded_unmodified() {
    let mut broken = delta_frame(r#"{"text": "broken"}"#);
    let last = broken.len().saturating_sub(1);
    if let Some(byte) = broken.get_mut(last) {
        *byte ^= 0xff;
    }
    let body = [broken.clone(), delta_frame(r#"{"text": "hello"}"#)].concat();
    let hook = Hook::replacing(&[("hello", "HELLO")]);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output.get(0..broken.len()), Some(broken.as_slice()));
    let parsed = parse_event_stream(&output);
    assert!(parsed.frames[0].texts.is_empty());
}

#[tokio::test]
async fn a_content_block_delta_with_a_payload_that_is_not_json_is_forwarded() {
    let broken = raw_frame(&[(":event-type", "contentBlockDelta")], b"not json");
    let body = [broken.clone(), delta_frame(r#"{"text": "hello"}"#)].concat();
    let hook = Hook::replacing(&[("hello", "HELLO")]);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output.get(0..broken.len()), Some(broken.as_slice()));
}

#[tokio::test]
async fn a_content_block_delta_whose_payload_is_not_an_object_is_forwarded() {
    let broken = raw_frame(&[(":event-type", "contentBlockDelta")], b"[1, 2]");
    let body = [broken.clone(), delta_frame(r#"{"text": "hello"}"#)].concat();
    let hook = Hook::replacing(&[("hello", "HELLO")]);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output.get(0..broken.len()), Some(broken.as_slice()));
}

#[tokio::test]
async fn trailing_bytes_are_kept_after_the_last_frame() {
    let body = [delta_frame(r#"{"text": "hello"}"#), b"garbage".to_vec()].concat();
    let hook = Hook::replacing(&[("hello", "HELLO")]);
    let output = anonymize_with(&body, &hook).await;
    let parsed = parse_event_stream(&output);
    assert_eq!(parsed.trailing, b"garbage".to_vec());
    assert!(output.ends_with(b"garbage"));
}

#[test]
fn a_truncated_frame_becomes_trailing_bytes() {
    let frame = delta_frame(r#"{"text": "hello"}"#);
    let truncated = frame.get(..5).expect("five bytes");
    let body = [frame.clone(), truncated.to_vec()].concat();
    let parsed = parse_event_stream(&body);
    assert_eq!(parsed.frames.len(), 1);
    assert_eq!(parsed.trailing, truncated);
}

#[test]
fn a_frame_length_below_the_prelude_stops_the_walk() {
    let mut body = 8_u32.to_be_bytes().to_vec();
    body.extend_from_slice(&[0_u8; 32]);
    let parsed = parse_event_stream(&body);
    assert!(parsed.frames.is_empty());
    assert_eq!(parsed.trailing, body);
}

#[test]
fn a_sixteen_byte_frame_that_ends_the_body_is_still_parsed() {
    let frame = raw_frame(&[], b"");
    assert_eq!(frame.len(), 16);
    let parsed = parse_event_stream(&frame);
    assert_eq!(parsed.frames.len(), 1);
    assert_eq!(parsed.frames[0].raw, frame);
    assert!(parsed.trailing.is_empty());
}

#[tokio::test]
async fn a_bad_length_before_a_valid_frame_keeps_it_in_the_trailing_bytes() {
    let tail = delta_frame(r#"{"toolUse": {"input": "body"}}"#);
    let mut trailing = 8_u32.to_be_bytes().to_vec();
    trailing.extend_from_slice(&tail);
    let body = [delta_frame(r#"{"text": "body"}"#), trailing.clone()].concat();
    let parsed = parse_event_stream(&body);
    assert_eq!(parsed.frames.len(), 1);
    assert_eq!(parsed.trailing, trailing);
    let hook = Hook::replacing(&[("body", "BODY")]);
    let output = anonymize_with(&body, &hook).await;
    assert!(output.ends_with(&trailing));
}

#[test]
fn value_type_name_names_each_json_kind() {
    assert_eq!(value_type_name(&GuardrailValue::Null), "null");
    assert_eq!(value_type_name(&GuardrailValue::Boolean(true)), "boolean");
    assert_eq!(value_type_name(&GuardrailValue::SignedInteger(1)), "number");
    assert_eq!(
        value_type_name(&GuardrailValue::UnsignedInteger(1)),
        "number"
    );
    assert_eq!(value_type_name(&GuardrailValue::Decimal(1.5)), "number");
    assert_eq!(
        value_type_name(&GuardrailValue::Text(String::new())),
        "string"
    );
    assert_eq!(value_type_name(&GuardrailValue::Array(Vec::new())), "list");
    assert_eq!(
        value_type_name(&GuardrailValue::Object(BTreeMap::new())),
        "dict"
    );
}

#[tokio::test]
async fn an_answer_that_is_not_an_object_leaves_the_stream_unmodified() {
    let body = delta_frame(r#"{"text": "hello"}"#);
    let hook = Hook::with_mode(Mode::NotAnObject);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output, body);
}

#[tokio::test]
async fn a_missing_content_block_leaves_the_stream_unmodified() {
    let body = [
        delta_frame(r#"{"text": "one"}"#),
        delta_frame(r#"{"text": "two"}"#),
    ]
    .concat();
    let hook = Hook::with_mode(Mode::DropFirstBlock);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output, body);
}

#[tokio::test]
async fn a_block_text_that_is_not_a_string_leaves_the_stream_unmodified() {
    let body = delta_frame(r#"{"text": "hello"}"#);
    let hook = Hook::with_mode(Mode::NonTextBlock);
    let output = anonymize_with(&body, &hook).await;
    assert_eq!(output, body);
}

#[tokio::test]
async fn a_hook_error_is_returned() {
    let body = delta_frame(r#"{"text": "hello"}"#);
    let hook = Hook::with_mode(Mode::Fail);
    let error = de_anonymize_event_stream(&body, &hook, &BTreeMap::new(), &BTreeMap::new())
        .await
        .expect_err("the hook fails");
    assert!(matches!(error, EventStreamError::Hook(HookFailure)));
}

#[test]
fn redistribute_rounds_halves_to_even() {
    let frames = vec![
        Frame {
            raw: Vec::new(),
            texts: vec![(GroupKey::Text, "a".to_owned())],
        },
        Frame {
            raw: Vec::new(),
            texts: vec![(GroupKey::Text, "b".to_owned())],
        },
    ];
    let groups = group_texts(&frames);
    assert_eq!(
        redistribute(&groups, &["12345".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 0), "12".to_owned()), ((1, 0), "345".to_owned())])
    );
    assert_eq!(
        redistribute(&groups, &["1234567".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 0), "1234".to_owned()), ((1, 0), "567".to_owned())])
    );
}

#[test]
fn redistribute_counts_characters_not_bytes() {
    // "é" is one character over two bytes, so a byte count gives the first member two of three characters
    let frames = vec![Frame {
        raw: Vec::new(),
        texts: vec![
            (GroupKey::Text, "\u{e9}".to_owned()),
            (GroupKey::Text, "ab".to_owned()),
        ],
    }];
    let groups = group_texts(&frames);
    assert_eq!(
        redistribute(&groups, &["xyz".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 0), "x".to_owned()), ((0, 1), "yz".to_owned())])
    );
}

#[test]
fn redistribute_clamps_a_shorter_replacement_and_gives_a_single_member_all() {
    let two = vec![Frame {
        raw: Vec::new(),
        texts: vec![
            (GroupKey::Text, "aaaa".to_owned()),
            (GroupKey::Text, "bbbb".to_owned()),
        ],
    }];
    assert_eq!(
        redistribute(&group_texts(&two), &["xy".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 0), "x".to_owned()), ((0, 1), "y".to_owned())])
    );
    let one = vec![Frame {
        raw: Vec::new(),
        texts: vec![(GroupKey::Text, "ab".to_owned())],
    }];
    assert_eq!(
        redistribute(&group_texts(&one), &["whole".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 0), "whole".to_owned())])
    );
}

#[test]
fn redistribute_skips_inactive_groups() {
    let frames = vec![Frame {
        raw: Vec::new(),
        texts: vec![
            (GroupKey::Text, String::new()),
            (GroupKey::Tool, "used".to_owned()),
        ],
    }];
    let groups = group_texts(&frames);
    assert_eq!(
        redistribute(&groups, &["REPLACED".to_owned()]).expect("shares"),
        BTreeMap::from([((0, 1), "REPLACED".to_owned())])
    );
}

#[test]
fn frames_without_texts_are_copied_verbatim() {
    let start = event_frame("messageStart", b"{\"role\": \"assistant\"}");
    let body = [start.clone(), delta_frame(r#"{"text": "hello"}"#)].concat();
    let parsed = parse_event_stream(&body);
    let output = rebuild_event_stream(&parsed, &BTreeMap::new());
    assert_eq!(output.get(0..start.len()), Some(start.as_slice()));
}

#[test]
fn a_rebuilt_frame_keeps_its_headers_and_writes_a_compact_payload() {
    let body = delta_frame(r#"{"text": "hello", "extra": 1}"#);
    let parsed = parse_event_stream(&body);
    let new_texts = BTreeMap::from([((0, 0), "HELLO".to_owned())]);
    let output = rebuild_event_stream(&parsed, &new_texts);
    let (headers, payload) = frame_parts(&output);
    assert_eq!(headers, frame_parts(&body).0);
    assert!(!payload.contains(&b' '));
    assert_eq!(
        serde_json::from_slice::<GuardrailValue>(&payload).expect("valid JSON"),
        serde_json::from_str::<GuardrailValue>(r#"{"delta":{"extra":1,"text":"HELLO"}}"#)
            .expect("valid JSON")
    );
}

#[test]
fn a_rebuilt_frame_has_valid_crcs_and_lengths() {
    let body = delta_frame(r#"{"text": "hello"}"#);
    let parsed = parse_event_stream(&body);
    let new_texts = BTreeMap::from([((0, 0), "a much longer replacement".to_owned())]);
    let output = rebuild_event_stream(&parsed, &new_texts);
    let message = read_message_from(output.as_slice()).expect("both CRCs validate");
    assert_eq!(message.headers().len(), 2);
    let length = u32::from_be_bytes(output.get(0..4).expect("prelude").try_into().expect("four"));
    assert_eq!(usize::try_from(length).expect("fits"), output.len());
}

#[test]
fn a_frame_whose_payload_does_not_parse_is_copied() {
    let broken = raw_frame(&[(":event-type", "contentBlockDelta")], b"not json");
    let frame = Frame {
        raw: broken.clone(),
        texts: vec![(GroupKey::Text, "something".to_owned())],
    };
    let parsed = ParsedStream {
        frames: vec![frame],
        trailing: Vec::new(),
    };
    let output = rebuild_event_stream(&parsed, &BTreeMap::new());
    assert_eq!(output, broken);
}

fn replay(body: &[u8]) {
    let parsed = parse_event_stream(body);
    let groups = group_texts(&parsed.frames);
    let returned: Vec<String> = groups
        .groups
        .iter()
        .map(|group| group.fragments.concat().to_uppercase())
        .collect();
    let new_texts = redistribute(&groups, &returned).expect("shares");
    drop(rebuild_event_stream(&parsed, &new_texts));
}

#[test]
fn replays_the_bedrock_event_stream_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_event_stream");
    let count = fs::read_dir(&root)
        .expect("corpus folder")
        .map(|entry| {
            let path = entry.expect("corpus entry").path();
            replay(&fs::read(&path).expect("corpus file"));
        })
        .count();
    assert!(count > 0);
}
