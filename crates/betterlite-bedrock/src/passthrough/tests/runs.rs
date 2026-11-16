use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, StreamChoice, StreamChunk, StreamDelta,
};

use super::super::chunks::{CoalescedChunks, plain_text_delta};
use super::{fixture_now, json};

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

pub(super) fn text_run_sealing() {
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
    assert_eq!(
        contents(chunks.chunks()),
        vec![Some("ab".to_owned()), Some("c".to_owned())]
    );
    let mut single = CoalescedChunks::default();
    single.add(text_chunk("only", "m", "x", None));
    assert_eq!(contents(single.chunks()), vec![Some("x".to_owned())]);
    let mut empty = CoalescedChunks::default();
    empty.add(text_chunk("e", "m", "", None));
    empty.add(text_chunk("e2", "m", "", None));
    assert_eq!(contents(empty.chunks()), vec![Some(String::new())]);
    let mut mixed = CoalescedChunks::default();
    mixed.add(text_chunk("t", "m", "a", None));
    mixed.add(text_chunk("t2", "m", "b", None));
    let mut tool = text_chunk("tool", "m", "x", None);
    tool.choices[0].delta.tool_calls = Some(json(r#"[{"index":0}]"#));
    mixed.add(tool);
    mixed.add(text_chunk("f", "m", "", None));
    let mut finished = text_chunk("finish", "m", "", None);
    finished.choices[0].finish_reason = Some("stop".to_owned());
    mixed.add(finished);
    mixed.add(text_chunk("after", "m", "z", None));
    let sealed = mixed.chunks();
    assert_eq!(
        contents(sealed),
        vec![
            Some("ab".to_owned()),
            Some("x".to_owned()),
            Some(String::new()),
            Some(String::new()),
            Some("z".to_owned()),
        ]
    );
    assert_eq!(sealed[0].id, "t");
    assert_eq!(sealed[1].id, "tool");
    assert_eq!(sealed[2].id, "f");
    assert_eq!(sealed[3].id, "finish");
    assert_eq!(sealed[4].id, "after");
}

fn parsed_chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).unwrap()
}

pub(super) fn plain_text_delta_rules() {
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"role":"assistant","content":"a"}}]}"#
        )),
        Some("a")
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(r#"{"choices":[{"delta":{"content":""}}]}"#)),
        Some("")
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(r#"{"choices":[{"delta":{}}]}"#)),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"role":"assistant"}}]}"#
        )),
        None
    );
    assert_eq!(plain_text_delta(&parsed_chunk(r#"{"choices":[]}"#)), None);
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"usage":{"total_tokens":1},"choices":[{"delta":{"content":"a"}}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"usage":{},"choices":[{"delta":{"content":"a"}}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"provider_specific_fields":{"x":1},"choices":[{"delta":{"content":"a"}}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"provider_specific_fields":{},"choices":[{"delta":{"content":"a"}}]}"#
        )),
        Some("a")
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"}},{"delta":{"content":"b"}}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"},"finish_reason":"stop"}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"},"finish_reason":""}]}"#
        )),
        Some("a")
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"},"finish_reason":null}]}"#
        )),
        Some("a")
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"},"logprobs":{}}]}"#
        )),
        None
    );
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a"},"logprobs":null}]}"#
        )),
        Some("a")
    );
    let blocked = [
        r#"{"content":"a","tool_calls":[]}"#,
        r#"{"content":"a","function_call":{"name":"f"}}"#,
        r#"{"content":"a","audio":{"id":"x"}}"#,
        r#"{"content":"a","images":[]}"#,
        r#"{"content":"a","annotations":[]}"#,
        r#"{"content":"a","reasoning_content":"r"}"#,
        r#"{"content":"a","thinking_blocks":[]}"#,
        r#"{"content":"a","provider_specific_fields":{}}"#,
        r#"{"content":"a","refusal":"no"}"#,
    ];
    blocked
        .iter()
        .try_for_each(|delta| {
            let source = format!(r#"{{"choices":[{{"delta":{delta}}}]}}"#);
            assert_eq!(plain_text_delta(&parsed_chunk(&source)), None, "{delta}");
            Ok::<(), ()>(())
        })
        .unwrap();
    assert_eq!(
        plain_text_delta(&parsed_chunk(
            r#"{"choices":[{"delta":{"content":"a","refusal":null}}]}"#
        )),
        Some("a")
    );
}
