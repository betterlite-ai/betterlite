use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

use super::support;

const CHAT: &str = r#"{"choices":[{"message":{"content":"thinking","tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}}]}"#;
const ANTHROPIC: &str = r#"{"content":[{"type":"text","text":"thinking"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
const RESPONSES: &str = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"thinking"}]},{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}]}"#;

async fn plan(response: &str) -> AgenticLoopPlan {
    let server = support::start(vec![
        support::compress_reply(&[support::HASH]),
        support::retrieve_reply("original text"),
    ])
    .await;
    let harness = support::harness(&server.base);
    harness.seed(support::CALL_ID, &[support::HASH]).await;
    let call = support::loop_call(response);
    support::build_plan(&harness, &call).await
}

#[tokio::test]
async fn an_openai_response_gets_the_assistant_and_tool_turns() {
    let messages = support::plan_messages(&plan(CHAT).await);
    assert_eq!(
        messages,
        vec![
            support::json(r#"{"role":"user","content":"hello"}"#),
            support::json(
                r#"{"role":"assistant","content":"thinking","tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}"#
            ),
            support::json(r#"{"role":"tool","tool_call_id":"toolu_1","content":"original text"}"#),
        ]
    );
}

#[tokio::test]
async fn an_anthropic_response_gets_tool_use_and_tool_result() {
    let messages = support::plan_messages(&plan(ANTHROPIC).await);
    assert_eq!(
        messages,
        vec![
            support::json(r#"{"role":"user","content":"hello"}"#),
            support::json(
                r#"{"role":"assistant","content":[{"type":"text","text":"thinking"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#
            ),
            support::json(
                r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"original text"}]}"#
            ),
        ]
    );
}

#[tokio::test]
async fn an_anthropic_response_without_text_has_only_the_tool_use() {
    let response = r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#
        )
    );
}

#[tokio::test]
async fn an_empty_anthropic_text_block_is_dropped() {
    let response = r#"{"content":[{"type":"text","text":""},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#
        )
    );
}

#[tokio::test]
async fn anthropic_text_blocks_are_joined_without_a_separator() {
    let response = r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":[{"type":"text","text":"ab"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#
        )
    );
}

#[tokio::test]
async fn a_responses_response_gets_function_call_items() {
    let messages = support::plan_messages(&plan(RESPONSES).await);
    assert_eq!(
        messages,
        vec![
            support::json(r#"{"role":"user","content":"hello"}"#),
            support::json(r#"{"role":"assistant","content":"thinking"}"#),
            support::json(
                r#"{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}"#
            ),
            support::json(
                r#"{"type":"function_call_output","call_id":"toolu_1","output":"original text"}"#
            ),
        ]
    );
}

#[tokio::test]
async fn a_responses_response_without_text_skips_the_assistant_item() {
    let response = r#"{"output":[{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}"#
        )
    );
}

#[tokio::test]
async fn an_empty_output_text_omits_the_assistant_item() {
    let response =
        r#"{"output":[{"type":"message","content":[{"type":"output_text","text":""}]}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}"#
        )
    );
}

#[tokio::test]
async fn only_output_text_chunks_are_read() {
    let response = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"a"},{"type":"input_text","text":"b"},{"type":"output_text","text":"c"}]}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(r#"{"role":"assistant","content":"ac"}"#)
    );
}

#[tokio::test]
async fn the_responses_shape_wins_when_both_lists_are_present() {
    let response = r#"{"output":[],"content":[{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"type":"function_call","call_id":"toolu_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}"#
        )
    );
}

#[tokio::test]
async fn the_anthropic_shape_wins_over_chat() {
    let response = r#"{"choices":[{"message":{"content":"chat text"}}],"content":[{"type":"text","text":"anthropic text"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":[{"type":"text","text":"chat text"},{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#
        )
    );
}

#[tokio::test]
async fn multimodal_chat_content_is_joined_with_blank_lines() {
    let response = r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"data:,"}},{"type":"text","text":5},{"type":"text","text":"b"}]}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":"a\n\nb","tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}"#
        )
    );
}

#[tokio::test]
async fn a_chat_response_without_text_has_null_content() {
    let response = r#"{"choices":[{"message":{"content":null}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":null,"tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}"#
        )
    );
}

#[tokio::test]
async fn an_empty_chat_text_has_null_content() {
    let response = r#"{"choices":[{"message":{"content":""}}]}"#;
    let messages = support::plan_messages(&plan(response).await);
    assert_eq!(
        messages[1],
        support::json(
            r#"{"role":"assistant","content":null,"tool_calls":[{"id":"toolu_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}"#
        )
    );
}

#[tokio::test]
async fn several_retrieve_calls_are_answered_in_order() {
    let server = support::start(vec![
        support::compress_reply(&[support::HASH, support::OTHER_HASH]),
        support::retrieve_reply("first"),
        support::retrieve_reply("second"),
    ])
    .await;
    let harness = support::harness(&server.base);
    harness
        .seed(support::CALL_ID, &[support::HASH, support::OTHER_HASH])
        .await;
    let mut call = support::loop_call(CHAT);
    call.tools = support::gate(&[
        support::tool_call("toolu_1", r#"{"hash":"abcdef123456"}"#),
        support::tool_call("toolu_2", r#"{"hash":"0123456789ab"}"#),
    ]);
    let messages = support::plan_messages(&support::build_plan(&harness, &call).await);
    assert_eq!(
        messages[2],
        support::json(r#"{"role":"tool","tool_call_id":"toolu_1","content":"first"}"#)
    );
    assert_eq!(
        messages[3],
        support::json(r#"{"role":"tool","tool_call_id":"toolu_2","content":"second"}"#)
    );
}
