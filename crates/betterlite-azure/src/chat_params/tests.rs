#![cfg(test)]

use rstest::rstest;

mod allowlist;
mod content;
mod lookup;
mod mapping;
mod stream;
mod support;

#[rstest]
#[case::gpt5_alias_catalog(allowlist::gpt5_alias_catalog)]
#[case::o3_native_stream(stream::o3_native_stream)]
#[case::unknown_o1_fake_stream(stream::unknown_o1_fake_stream)]
#[case::gpt5_base_policy_composition(content::gpt5_base_policy_composition)]
#[case::o_series_base_policy_composition(allowlist::o_series_base_policy_composition)]
#[case::azure_gpt5_allowlist(allowlist::azure_gpt5_allowlist)]
#[case::azure_effort_none_drop(mapping::azure_effort_none_drop)]
#[case::azure_reasoning_route_strip(content::azure_reasoning_route_strip)]
fn chat_params_cases(#[case] scenario: fn()) {
    scenario();
}
