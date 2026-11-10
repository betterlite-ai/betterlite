use std::iter::repeat_with;

use crate::anthropic::assistant::{AssistantError, MAX_PROVIDER_BLOCKS};
use crate::anthropic::error::AnthropicPromptError;
use crate::anthropic_history::tools::MAX_HISTORY_MESSAGES;
use crate::anthropic_invokes::invoke::MAX_TOOL_CALLS;
use crate::anthropic_user_blocks::prelude::PrepareError;
use crate::arguments::json::Json;
use crate::media::limits::MAX_MESSAGE_PARTS;

use super::fixtures::{MODEL, NoFileBlocksError, convert_values, expected};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn thinking_block(index: usize) -> Json {
    object(vec![
        ("type", text("thinking")),
        ("thinking", text(&format!("t{index}"))),
        ("signature", text(&format!("s{index}"))),
    ])
}

fn compaction_block() -> Json {
    object(vec![
        ("type", text("compaction")),
        ("content", text("summary")),
        ("signature", text("sig")),
    ])
}

fn user_turn() -> Json {
    object(vec![("role", text("user")), ("content", text("u"))])
}

fn assistant_turn(entries: Vec<(&str, Json)>) -> Json {
    let mut fields: Vec<(&str, Json)> = vec![("role", text("assistant")), ("content", text("a"))];
    fields.extend(entries);
    object(fields)
}

async fn convert_message(
    messages: Vec<Json>,
) -> Result<Vec<Json>, AnthropicPromptError<NoFileBlocksError>> {
    convert_values(&messages, MODEL, false, &[]).await
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_thinking_blocks_at_the_bound_convert() {
    let blocks: Vec<Json> = (0..MAX_PROVIDER_BLOCKS).map(thinking_block).collect();
    let messages = vec![
        user_turn(),
        assistant_turn(vec![("thinking_blocks", Json::List(blocks))]),
    ];
    assert_eq!(convert_message(messages).await.unwrap().len(), 2);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_thinking_blocks_over_the_bound_are_refused()
 {
    let blocks: Vec<Json> = (0..MAX_PROVIDER_BLOCKS.saturating_add(1))
        .map(thinking_block)
        .collect();
    let messages = vec![
        user_turn(),
        assistant_turn(vec![("thinking_blocks", Json::List(blocks))]),
    ];
    let error = convert_message(messages)
        .await
        .expect_err("the bound is enforced");
    assert!(matches!(
        error,
        AnthropicPromptError::Assistant(AssistantError::TooMany {
            index: 1,
            field: "thinking_blocks",
            limit: MAX_PROVIDER_BLOCKS,
        })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_compaction_blocks_at_the_bound_convert()
{
    let blocks = vec![compaction_block(); MAX_PROVIDER_BLOCKS];
    let messages = vec![
        user_turn(),
        assistant_turn(vec![(
            "provider_specific_fields",
            object(vec![("compaction_blocks", Json::List(blocks))]),
        )]),
    ];
    assert_eq!(convert_message(messages).await.unwrap().len(), 2);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_compaction_blocks_over_the_bound_are_refused()
 {
    let blocks = vec![compaction_block(); MAX_PROVIDER_BLOCKS.saturating_add(1)];
    let messages = vec![
        user_turn(),
        assistant_turn(vec![(
            "provider_specific_fields",
            object(vec![("compaction_blocks", Json::List(blocks))]),
        )]),
    ];
    let error = convert_message(messages)
        .await
        .expect_err("the bound is enforced");
    assert!(matches!(
        error,
        AnthropicPromptError::Assistant(AssistantError::TooMany {
            index: 1,
            field: "compaction_blocks",
            limit: MAX_PROVIDER_BLOCKS,
        })
    ));
}

fn skipped_call(index: usize) -> Json {
    object(vec![
        ("id", text(&format!("call_{index}"))),
        ("type", text("custom")),
    ])
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_tool_calls_at_the_bound_convert() {
    let calls: Vec<Json> = (0..MAX_TOOL_CALLS).map(skipped_call).collect();
    let messages = vec![
        user_turn(),
        assistant_turn(vec![("tool_calls", Json::List(calls))]),
    ];
    assert_eq!(
        convert_message(messages).await.unwrap(),
        expected(
            r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#
        )
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_tool_calls_over_the_bound_are_refused() {
    let calls: Vec<Json> = (0..MAX_TOOL_CALLS.saturating_add(1))
        .map(skipped_call)
        .collect();
    let messages = vec![
        user_turn(),
        assistant_turn(vec![("tool_calls", Json::List(calls))]),
    ];
    let error = convert_message(messages)
        .await
        .expect_err("the bound is enforced");
    assert!(matches!(
        error,
        AnthropicPromptError::Assistant(AssistantError::TooMany {
            index: 1,
            field: "tool_calls",
            limit: MAX_TOOL_CALLS,
        })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_content_parts_over_the_bound_are_refused()
 {
    let parts = vec![
        object(vec![("type", text("text")), ("text", text("x"))]);
        MAX_MESSAGE_PARTS.saturating_add(1)
    ];
    let messages = vec![object(vec![
        ("role", text("user")),
        ("content", Json::List(parts)),
    ])];
    let error = convert_message(messages)
        .await
        .expect_err("the bound is enforced");
    assert!(matches!(
        error,
        AnthropicPromptError::Prepare(PrepareError::TooManyParts {
            limit: MAX_MESSAGE_PARTS
        })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_messages_over_the_bound_are_refused() {
    let messages: Vec<Json> = repeat_with(user_turn)
        .take(MAX_HISTORY_MESSAGES.saturating_add(1))
        .collect();
    let error = convert_message(messages)
        .await
        .expect_err("the bound is enforced");
    assert!(matches!(
        error,
        AnthropicPromptError::Prepare(PrepareError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES
        })
    ));
}
