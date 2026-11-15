#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::mantle_api_base(cases::mantle_api_base)]
#[case::native_responses(cases::native_responses)]
#[case::cache_point_unknown_model(cases::cache_point_unknown_model)]
#[case::strict_tools_fallback(cases::strict_tools_fallback)]
#[case::tool_search(cases::tool_search)]
#[case::effort_ceiling(cases::effort_ceiling)]
#[case::parallel_tools(cases::parallel_tools)]
#[case::extended_cache_ttl(cases::extended_cache_ttl)]
fn bedrock_model_capabilities_cases(#[case] case: fn()) {
    case();
}
