use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk, StreamDelta,
};

use super::super::chunks::{CoalescedChunks, plain_text_delta};
use super::fixture_now;

const TEXT_RULES: [(&str, Option<&str>); 17] = [
    (
        r#"{"choices":[{"delta":{"role":"assistant","content":"a"}}]}"#,
        Some("a"),
    ),
    (r#"{"choices":[{"delta":{"content":""}}]}"#, Some("")),
    (r#"{"choices":[{"delta":{}}]}"#, None),
    (r#"{"choices":[{"delta":{"role":"assistant"}}]}"#, None),
    (r#"{"choices":[]}"#, None),
    (
        r#"{"usage":{"total_tokens":1},"choices":[{"delta":{"content":"a"}}]}"#,
        None,
    ),
    (
        r#"{"usage":{},"choices":[{"delta":{"content":"a"}}]}"#,
        None,
    ),
    (
        r#"{"provider_specific_fields":{"x":1},"choices":[{"delta":{"content":"a"}}]}"#,
        None,
    ),
    (
        r#"{"provider_specific_fields":{},"choices":[{"delta":{"content":"a"}}]}"#,
        Some("a"),
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"}},{"delta":{"content":"b"}}]}"#,
        None,
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"},"finish_reason":"stop"}]}"#,
        None,
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"},"finish_reason":""}]}"#,
        Some("a"),
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"},"finish_reason":null}]}"#,
        Some("a"),
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"},"logprobs":{}}]}"#,
        None,
    ),
    (
        r#"{"choices":[{"delta":{"content":"a"},"logprobs":null}]}"#,
        Some("a"),
    ),
    (
        r#"{"choices":[{"delta":{"content":"a","refusal":null}}]}"#,
        Some("a"),
    ),
    (
        r#"{"choices":[{"delta":{"content":"a","refusal":"no"}}]}"#,
        None,
    ),
];

const BLOCKED_DELTA_FIELDS: [&str; 8] = [
    r#""tool_calls":[]"#,
    r#""function_call":{"name":"f"}"#,
    r#""audio":{"id":"x"}"#,
    r#""images":[]"#,
    r#""annotations":[]"#,
    r#""reasoning_content":"r""#,
    r#""thinking_blocks":[]"#,
    r#""provider_specific_fields":{}"#,
];

fn empty_delta(content: Option<&str>, role: Option<&str>) -> StreamDelta {
    StreamDelta {
        content: content.map(str::to_owned),
        role: role.map(str::to_owned),
        function_call: None,
        tool_calls: None,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content: None,
        thinking_blocks: None,
        provider_specific_fields: None,
        extra: JsonObject::new(),
    }
}

fn text_chunk(id: &str, model: &str, content: &str, role: Option<&str>) -> StreamChunk {
    StreamChunk {
        id: id.to_owned(),
        object: "chat.completion.chunk".to_owned(),
        created: fixture_now(),
        model: model.to_owned(),
        system_fingerprint: None,
        choices: vec![StreamChoice {
            index: 0,
            delta: empty_delta(Some(content), role),
            finish_reason: None,
            logprobs: None,
            extra: JsonObject::new(),
        }],
        usage: None,
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn contents(chunks: &[StreamChunk]) -> Vec<Option<String>> {
    chunks
        .iter()
        .map(|chunk| chunk.choices[0].delta.content.clone())
        .collect()
}

fn texts(parts: &[&str]) -> Vec<Option<String>> {
    parts.iter().map(|part| Some((*part).to_owned())).collect()
}

fn parsed_chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).unwrap()
}

fn first_chunk_keeps_its_fields() {
    let mut chunks = CoalescedChunks::default();
    chunks.add(text_chunk("c1", "m1", "a", Some("assistant")));
    chunks.add(text_chunk("c2", "m2", "", None));
    chunks.add(text_chunk("c3", "m3", "b", None));
    let sealed = chunks.chunks();
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].id, "c1");
    assert_eq!(sealed[0].model, "m1");
    assert_eq!(
        sealed[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(sealed[0].choices[0].delta.content.as_deref(), Some("ab"));
    chunks.add(text_chunk("c4", "m", "c", None));
    assert_eq!(contents(chunks.chunks()), texts(&["ab", "c"]));
}

fn single_and_empty_runs() {
    let mut single = CoalescedChunks::default();
    single.add(text_chunk("only", "m", "x", None));
    assert_eq!(contents(single.chunks()), texts(&["x"]));
    let mut empty = CoalescedChunks::default();
    empty.add(text_chunk("e", "m", "", None));
    empty.add(text_chunk("e2", "m", "", None));
    assert_eq!(contents(empty.chunks()), texts(&[""]));
}

fn nontext_events_seal_in_order() {
    let mut mixed = CoalescedChunks::default();
    mixed.add(text_chunk("t", "m", "a", None));
    mixed.add(text_chunk("t2", "m", "b", None));
    let mut tool = text_chunk("tool", "m", "x", None);
    tool.choices[0].delta.tool_calls = Some(JsonValue::Array(Vec::new()));
    mixed.add(tool);
    mixed.add(text_chunk("f", "m", "", None));
    let mut finished = text_chunk("finish", "m", "", None);
    finished.choices[0].finish_reason = Some("stop".to_owned());
    mixed.add(finished);
    mixed.add(text_chunk("after", "m", "z", None));
    let all = mixed.chunks();
    assert_eq!(contents(all), texts(&["ab", "x", "", "", "z"]));
    let ids: Vec<&str> = all.iter().map(|chunk| chunk.id.as_str()).collect();
    assert_eq!(ids, ["t", "tool", "f", "finish", "after"]);
}

pub(super) fn text_run_sealing() {
    first_chunk_keeps_its_fields();
    single_and_empty_runs();
    nontext_events_seal_in_order();
}

pub(super) fn plain_text_delta_rules() {
    for (source, expected) in TEXT_RULES {
        assert_eq!(
            plain_text_delta(&parsed_chunk(source)),
            expected,
            "{source}"
        );
    }
    for field in BLOCKED_DELTA_FIELDS {
        let source = format!(r#"{{"choices":[{{"delta":{{"content":"a",{field}}}}}]}}"#);
        assert_eq!(plain_text_delta(&parsed_chunk(&source)), None, "{field}");
    }
}
