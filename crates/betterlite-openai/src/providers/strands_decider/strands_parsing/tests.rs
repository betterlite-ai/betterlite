#![cfg(test)]

use rstest::rstest;

use super::arn::agentcore_runtime;

#[rstest]
#[case::commercial(
    "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime",
    "us-east-1"
)]
#[case::china(
    "arn:aws-cn:bedrock-agentcore:cn-north-1:123456789012:runtime/r",
    "cn-north-1"
)]
#[case::governance(
    "arn:aws-us-gov:bedrock-agentcore:us-gov-west-1:123456789012:runtime/r",
    "us-gov-west-1"
)]
#[case::iso_b(
    "arn:aws-iso-b:bedrock-agentcore:us-isob-east-1:123456789012:runtime/r",
    "us-isob-east-1"
)]
#[case::iso_e(
    "arn:aws-iso-e:bedrock-agentcore:eu-isoe-west-1:123456789012:runtime/r",
    "eu-isoe-west-1"
)]
#[case::iso_f(
    "arn:aws-iso-f:bedrock-agentcore:us-isof-south-1:123456789012:runtime/r",
    "us-isof-south-1"
)]
#[case::two_partition_groups(
    "arn:aws-cn-cn:bedrock-agentcore:us-east-1:123456789012:runtime/r",
    "us-east-1"
)]
#[case::runtime_name_characters(
    "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/a_b-c9",
    "us-east-1"
)]
#[case::region_of_dashes("arn:aws:bedrock-agentcore:-:123456789012:runtime/r", "-")]
fn test_arn_parsing_accepts_runtime_arns(#[case] api_base: &str, #[case] region: &str) {
    let runtime = agentcore_runtime(api_base).expect("a runtime ARN");
    assert_eq!(runtime.region, region);
    assert_eq!(runtime.arn, api_base);
}

#[rstest]
#[case::partition_digit("arn:aws1:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::partition_group_without_dash(
    "arn:awscn:bedrock-agentcore:us-east-1:123456789012:runtime/r"
)]
#[case::partition_group_digit("arn:aws-1:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::partition_empty_group("arn:aws-:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::partition_double_dash("arn:aws--cn:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::short_account("arn:aws:bedrock-agentcore:us-east-1:12345678901:runtime/r")]
#[case::long_account("arn:aws:bedrock-agentcore:us-east-1:1234567890123:runtime/r")]
#[case::letter_account("arn:aws:bedrock-agentcore:us-east-1:12345678901a:runtime/r")]
#[case::uppercase_region("arn:aws:bedrock-agentcore:US-EAST-1:123456789012:runtime/r")]
#[case::empty_region("arn:aws:bedrock-agentcore::123456789012:runtime/r")]
#[case::crafted_host("arn:aws:bedrock-agentcore:us-east-1.example.com:123456789012:runtime/r")]
#[case::other_service("arn:aws:bedrock:us-east-1:123456789012:runtime/r")]
#[case::other_resource("arn:aws:bedrock-agentcore:us-east-1:123456789012:endpoint/r")]
#[case::trailing_slash("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r/")]
#[case::nested_resource("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r/endpoint")]
#[case::dotted_runtime("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r.x")]
#[case::empty_runtime("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/")]
#[case::percent_encoded_runtime("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r%2Fy")]
#[case::trailing_space("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r ")]
#[case::leading_space(" arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::trailing_newline("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r\n")]
#[case::extra_field("arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r:extra")]
#[case::uppercase_scheme("ARN:aws:bedrock-agentcore:us-east-1:123456789012:runtime/r")]
#[case::invocations_url(
    "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Aus-east-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::empty("")]
fn test_arn_parsing_rejects_non_runtime_strings(#[case] api_base: &str) {
    assert!(agentcore_runtime(api_base).is_none(), "{api_base:?}");
}
