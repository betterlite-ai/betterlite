#![cfg(test)]

use futures_util::{StreamExt as _, stream};

use super::ChatGptToolCallNormalizer;
use crate::stream::chunk::{Choice, Chunk, Delta, Function, ToolCall};

fn tool_call(index: u32, id: Option<&str>) -> ToolCall {
    ToolCall {
        index,
        id: id.map(str::to_owned),
        kind: Some("function".to_owned()),
        function: Some(Function {
            name: Some("lookup".to_owned()),
            arguments: String::new(),
        }),
    }
}

fn delta(content: Option<&str>, tool_calls: Option<Vec<ToolCall>>) -> Delta {
    Delta {
        role: None,
        content: content.map(str::to_owned),
        refusal: None,
        tool_calls,
        function_call: None,
    }
}

fn choice(index: u32, delta: Delta, finish_reason: Option<&str>) -> Choice {
    Choice {
        index,
        delta,
        finish_reason: finish_reason.map(str::to_owned),
        logprobs: None,
    }
}

fn chunk(choices: Vec<Choice>) -> Chunk {
    Chunk {
        id: "chatcmpl-1".to_owned(),
        object: "chat.completion.chunk".to_owned(),
        created: 1,
        model: "gpt-5".to_owned(),
        choices,
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    }
}

fn indices(chunks: &[Chunk]) -> Vec<u32> {
    chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .flat_map(|choice| choice.delta.tool_calls.as_deref().unwrap_or_default())
        .map(|call| call.index)
        .collect()
}

fn interleaved_input() -> Vec<Chunk> {
    vec![
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("alpha"))])),
            None,
        )]),
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("beta"))])),
            None,
        )]),
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, None)])),
            None,
        )]),
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("alpha"))])),
            Some("tool_calls"),
        )]),
        chunk(vec![choice(0, delta(Some("text"), None), None)]),
    ]
}

#[test]
fn assigns_contiguous_indices_to_new_ids() {
    let input = vec![chunk(vec![choice(
        0,
        delta(
            None,
            Some(vec![
                tool_call(0, Some("alpha")),
                tool_call(0, Some("beta")),
            ]),
        ),
        None,
    )])];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(indices(&output), [0, 1]);
}

#[test]
fn drops_a_repeated_id() {
    let input = vec![
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("alpha"))])),
            None,
        )]),
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("alpha"))])),
            Some("tool_calls"),
        )]),
    ];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(output.len(), 1);
    assert_eq!(indices(&output), [0]);
}

#[test]
fn id_less_deltas_follow_the_last_seen_id() {
    let output: Vec<Chunk> =
        ChatGptToolCallNormalizer::from(interleaved_input().into_iter()).collect();
    assert_eq!(indices(&output), [0, 1, 1]);
}

#[test]
fn id_less_delta_keeps_its_index_before_any_id() {
    let input = vec![chunk(vec![choice(
        0,
        delta(None, Some(vec![tool_call(5, None)])),
        None,
    )])];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(indices(&output), [5]);
}

#[test]
fn chunks_with_empty_choices_pass_unchanged() {
    let input = vec![chunk(Vec::new())];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(output.len(), 1);
    assert!(output[0].choices.is_empty());
}

#[test]
fn chunks_without_tool_calls_pass_unchanged() {
    let input = vec![chunk(vec![choice(0, delta(Some("hello"), None), None)])];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].choices[0].delta.content.as_deref(), Some("hello"));
    assert!(output[0].choices[0].delta.tool_calls.is_none());
}

#[test]
fn empty_tool_calls_pass_unchanged() {
    let input = vec![chunk(vec![choice(0, delta(None, Some(Vec::new())), None)])];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(output.len(), 1);
    assert!(
        output[0].choices[0]
            .delta
            .tool_calls
            .as_ref()
            .is_some_and(Vec::is_empty)
    );
}

#[test]
fn duplicate_only_chunks_are_removed_with_content_and_finish_reason() {
    let input = vec![
        chunk(vec![choice(
            0,
            delta(None, Some(vec![tool_call(0, Some("alpha"))])),
            None,
        )]),
        chunk(vec![choice(
            0,
            delta(Some("closing"), Some(vec![tool_call(0, Some("alpha"))])),
            Some("stop"),
        )]),
        chunk(vec![choice(0, delta(Some("next"), None), None)]),
    ];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    assert_eq!(output.len(), 2);
    assert_eq!(output[1].choices[0].delta.content.as_deref(), Some("next"));
    assert!(output[1].choices[0].finish_reason.is_none());
}

#[test]
fn only_the_first_choice_is_normalized() {
    let input = vec![chunk(vec![
        choice(
            0,
            delta(None, Some(vec![tool_call(3, Some("alpha"))])),
            None,
        ),
        choice(
            1,
            delta(None, Some(vec![tool_call(9, Some("omega"))])),
            None,
        ),
    ])];
    let output: Vec<Chunk> = ChatGptToolCallNormalizer::from(input.into_iter()).collect();
    let first = output[0].choices[0]
        .delta
        .tool_calls
        .as_deref()
        .unwrap_or_default();
    let second = output[0].choices[1]
        .delta
        .tool_calls
        .as_deref()
        .unwrap_or_default();
    assert_eq!(first[0].index, 0);
    assert_eq!(second[0].index, 9);
    assert_eq!(second[0].id.as_deref(), Some("omega"));
}

fn single_alpha() -> Vec<Chunk> {
    vec![chunk(vec![choice(
        0,
        delta(None, Some(vec![tool_call(0, Some("alpha"))])),
        None,
    )])]
}

#[test]
fn separate_streams_keep_separate_state() {
    let mut first = ChatGptToolCallNormalizer::from(single_alpha().into_iter());
    let mut second = ChatGptToolCallNormalizer::from(single_alpha().into_iter());
    assert_eq!(first.next().map(|chunk| indices(&[chunk])), Some(vec![0]));
    assert_eq!(second.next().map(|chunk| indices(&[chunk])), Some(vec![0]));
    assert!(first.next().is_none());
    assert!(second.next().is_none());
}

#[tokio::test]
async fn async_iteration_normalizes_like_sync_iteration() {
    let sync_output: Vec<Chunk> =
        ChatGptToolCallNormalizer::from(interleaved_input().into_iter()).collect();
    let mut source = ChatGptToolCallNormalizer::from(stream::iter(interleaved_input()));
    let mut async_output = Vec::new();
    while let Some(chunk) = source.next().await {
        async_output.push(chunk);
    }
    assert_eq!(indices(&sync_output), indices(&async_output));
    assert_eq!(sync_output.len(), async_output.len());
}
