#![cfg(test)]

mod cases;
mod corpus;
mod counter;
mod support;

use rstest::rstest;

#[rstest]
#[case::qwen2_output(cases::qwen2_output::qwen2_output)]
#[case::qwen3_chat_prompt(cases::qwen3_chat_prompt::qwen3_chat_prompt)]
#[case::qwen3_options(cases::qwen3_options::qwen3_options)]
#[case::qwen3_reasoning_content(cases::qwen3_reasoning_content::qwen3_reasoning_content)]
#[case::deepseek_buffered_response(cases::deepseek_buffered_response::deepseek_buffered_response)]
#[case::qwen3_tool_prompt(cases::qwen3_tool_prompt::qwen3_tool_prompt)]
#[case::qwen_usage_and_failure(cases::qwen_usage_and_failure::qwen_usage_and_failure)]
fn qwen_deepseek_cases(#[case] case: fn()) {
    case();
}
