#![cfg(test)]

use rstest::rstest;

use crate::prometheus_latency::input_sequence_length::{
    InputLengthInputs, UsageMapping, input_sequence_length_bucket,
};
use crate::prometheus_latency::test_support::{bucket_inputs, combined, mapping};

#[rstest]
#[case::nonzero_prompt_tokens_is_first(
    bucket_inputs(Some(1500), None, false, UsageMapping::Absent, UsageMapping::Absent),
    "1k-4k"
)]
#[case::negative_prompt_tokens_is_unknown(
    bucket_inputs(Some(-5), None, false, UsageMapping::Absent, UsageMapping::Absent),
    "unknown"
)]
#[case::combined_usage_upstream_total_wins(
    bucket_inputs(
        Some(0),
        Some(combined(Some(1))),
        true,
        mapping(Some(5000)),
        mapping(Some(70000))
    ),
    "unknown"
)]
#[case::combined_usage_without_upstream_total_falls_through(
    bucket_inputs(
        Some(0),
        Some(combined(None)),
        true,
        mapping(Some(5000)),
        UsageMapping::Absent
    ),
    "4k-16k"
)]
#[case::missing_response_usage_comes_before_metadata(
    bucket_inputs(Some(0), None, false, mapping(Some(5000)), mapping(Some(70000))),
    "unknown"
)]
#[case::metadata_mapping_wins_over_the_response_object(
    bucket_inputs(None, None, true, mapping(Some(2000)), mapping(Some(70000))),
    "1k-4k"
)]
#[case::response_object_usage_is_used(
    bucket_inputs(None, None, true, UsageMapping::Absent, mapping(Some(70000))),
    "64k+"
)]
#[case::response_object_is_ignored_with_combined_usage(
    bucket_inputs(
        None,
        Some(combined(None)),
        true,
        UsageMapping::Absent,
        mapping(Some(70000))
    ),
    "unknown"
)]
#[case::zero_prompt_tokens_reaches_the_zero_bucket(
    bucket_inputs(Some(0), None, true, UsageMapping::Absent, UsageMapping::Absent),
    "0-1k"
)]
#[case::absent_prompt_tokens_is_unknown(
    bucket_inputs(None, None, true, UsageMapping::Absent, UsageMapping::Absent),
    "unknown"
)]
#[case::metadata_mapping_without_prompt_tokens(
    bucket_inputs(None, None, true, mapping(None), UsageMapping::Absent),
    "unknown"
)]
#[case::response_object_without_prompt_tokens(
    bucket_inputs(None, None, true, UsageMapping::Absent, mapping(None)),
    "unknown"
)]
fn input_sequence_length_follows_the_six_steps(
    #[case] inputs: InputLengthInputs,
    #[case] expected: &str,
) {
    assert_eq!(input_sequence_length_bucket(&inputs), expected);
}
