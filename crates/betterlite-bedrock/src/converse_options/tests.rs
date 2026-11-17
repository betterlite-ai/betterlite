#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::nova2_reasoning_config(cases::nova2_reasoning_config)]
#[case::gpt_reasoning_object(cases::gpt_reasoning_object)]
#[case::gpt_oss_and_deepseek_effort(cases::gpt_oss_and_deepseek_effort)]
#[case::anthropic_adaptive_effort(cases::anthropic_adaptive_effort)]
#[case::minimum_thinking_budget(cases::minimum_thinking_budget)]
#[case::nova_web_search_only(cases::nova_web_search_only)]
#[case::sampling_capabilities(cases::sampling_capabilities)]
#[case::supported_param_list(cases::supported_param_list)]
#[case::converse_class_defaults(cases::converse_class_defaults)]
fn converse_options_cases(#[case] case: fn()) {
    case();
}
