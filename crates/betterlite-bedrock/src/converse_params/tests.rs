#![cfg(test)]

mod map_cases;
mod scenario;
mod support;
mod thinking_cases;

use rstest::rstest;

#[rstest]
#[case::inference_config_options(map_cases::inference_config_options)]
#[case::additional_model_fields(map_cases::additional_model_fields)]
#[case::service_tier(map_cases::service_tier)]
#[case::request_metadata(map_cases::request_metadata)]
#[case::cache_point_ttl(map_cases::cache_point_ttl)]
#[case::top_k(map_cases::top_k)]
#[case::parallel_tool_config(map_cases::parallel_tool_config)]
#[case::thinking_budget_with_max_tokens(thinking_cases::thinking_budget_with_max_tokens)]
#[case::adaptive_thinking_fallback(thinking_cases::adaptive_thinking_fallback)]
#[case::final_forced_tool_downgrade(thinking_cases::final_forced_tool_downgrade)]
#[case::context_management(map_cases::context_management)]
fn converse_params_cases(#[case] case: fn()) {
    case();
}
