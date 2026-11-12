#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::assistant_text_from_response;

fn text_of(source: &str) -> Option<String> {
    let response: GuardrailValue = serde_json::from_str(source).unwrap();
    assistant_text_from_response(&response)
}

#[rstest]
#[case::chat_string(
    r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#,
    Some("hello")
)]
#[case::chat_text_parts(r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"image_url"},{"type":"text","text":"b"}]}}]}"#, Some("a\n\nb"))]
#[case::chat_only_the_first_choice(
    r#"{"choices":[{"message":{"content":"first"}},{"message":{"content":"second"}}]}"#,
    Some("first")
)]
#[case::chat_empty_content_falls_through(r#"{"choices":[{"message":{"content":""}}],"output":[{"type":"message","content":[{"type":"output_text","text":"from output"}]}]}"#, Some("from output"))]
#[case::chat_null_content(r#"{"choices":[{"message":{"content":null}}]}"#, None)]
#[case::chat_tool_call_message_has_no_text(
    r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"1"}]}}]}"#,
    None
)]
#[case::chat_no_message(r#"{"choices":[{"finish_reason":"stop"}]}"#, None)]
#[case::chat_null_message(r#"{"choices":[{"message":null}]}"#, None)]
#[case::chat_empty_choices(r#"{"choices":[]}"#, None)]
#[case::chat_choices_not_a_list(r#"{"choices":"x"}"#, None)]
#[case::chat_message_not_an_object(r#"{"choices":[{"message":"hi"}]}"#, None)]
#[case::chat_choice_not_an_object(r#"{"choices":["x"]}"#, None)]
#[case::chat_content_parts_without_text(
    r#"{"choices":[{"message":{"content":[{"type":"image_url"}]}}]}"#,
    None
)]
#[case::anthropic_one_block(r#"{"content":[{"type":"text","text":"hello"}]}"#, Some("hello"))]
#[case::anthropic_blocks_join_without_separator(r#"{"content":[{"type":"text","text":"a"},{"type":"tool_use","name":"t"},{"type":"text","text":"b"}]}"#, Some("ab"))]
#[case::anthropic_empty_blocks_are_dropped(
    r#"{"content":[{"type":"text","text":""},{"type":"text","text":"b"}]}"#,
    Some("b")
)]
#[case::anthropic_block_text_must_be_a_string(
    r#"{"content":[{"type":"text","text":5},{"type":"text"}]}"#,
    None
)]
#[case::anthropic_thinking_only(r#"{"content":[{"type":"thinking","thinking":"hmm"}]}"#, None)]
#[case::anthropic_block_without_type(r#"{"content":[{"text":"a"}]}"#, None)]
#[case::anthropic_blocks_not_objects(r#"{"content":["a"]}"#, None)]
#[case::anthropic_empty_list(r#"{"content":[]}"#, None)]
#[case::anthropic_content_is_a_string(r#"{"content":"hello"}"#, None)]
#[case::chat_wins_over_anthropic(r#"{"choices":[{"message":{"content":"chat"}}],"content":[{"type":"text","text":"anthropic"}]}"#, Some("chat"))]
#[case::anthropic_wins_over_responses(r#"{"content":[{"type":"text","text":"anthropic"}],"output":[{"type":"message","content":[{"type":"output_text","text":"responses"}]}]}"#, Some("anthropic"))]
#[case::responses_message_chunks(r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"a"},{"type":"output_text","text":"b"}]}]}"#, Some("ab"))]
#[case::responses_several_messages(r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"a"}]},{"type":"reasoning","content":[{"type":"output_text","text":"skip"}]},{"type":"message","content":[{"type":"output_text","text":"b"}]}]}"#, Some("ab"))]
#[case::responses_only_output_text_chunks(r#"{"output":[{"type":"message","content":[{"type":"refusal","text":"no"},{"type":"output_text","text":"yes"}]}]}"#, Some("yes"))]
#[case::responses_empty_chunk_text_is_dropped(
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":""}]}]}"#,
    None
)]
#[case::responses_content_missing(r#"{"output":[{"type":"message"}]}"#, None)]
#[case::responses_content_null(r#"{"output":[{"type":"message","content":null}]}"#, None)]
#[case::responses_content_is_a_string(r#"{"output":[{"type":"message","content":"hello"}]}"#, None)]
#[case::responses_function_call_only(r#"{"output":[{"type":"function_call","name":"f"}]}"#, None)]
#[case::responses_empty_output(r#"{"output":[]}"#, None)]
#[case::responses_output_not_a_list(r#"{"output":"x"}"#, None)]
#[case::empty_object("{}", None)]
#[case::response_is_a_string(r#""hello""#, None)]
#[case::response_is_null("null", None)]
fn assistant_text_comes_from_any_response_shape(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(text_of(source).as_deref(), expected);
}
