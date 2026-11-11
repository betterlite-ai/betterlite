#![cfg(test)]

use rstest::rstest;

use super::StrandsDeciderDecisionsConfig;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;

#[test]
fn strands_url_builds_the_agentcore_invocations_url_from_an_arn() {
    let config = StrandsDeciderDecisionsConfig::default();
    assert_eq!(
        config.get_complete_url(
            "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/my-runtime",
            "any-model"
        ),
        "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/\
         arn%3Aaws%3Abedrock-agentcore%3Aus-east-1%3A123456789012%3Aruntime%2Fmy-runtime/invocations"
    );
}

#[rstest]
#[case::china(
    "arn:aws-cn:bedrock-agentcore:cn-north-1:123456789012:runtime/r",
    "https://bedrock-agentcore.cn-north-1.amazonaws.com.cn/runtimes/\
     arn%3Aaws-cn%3Abedrock-agentcore%3Acn-north-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::governance(
    "arn:aws-us-gov:bedrock-agentcore:us-gov-west-1:123456789012:runtime/r",
    "https://bedrock-agentcore.us-gov-west-1.amazonaws.com/runtimes/\
     arn%3Aaws-us-gov%3Abedrock-agentcore%3Aus-gov-west-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::iso_b(
    "arn:aws-iso-b:bedrock-agentcore:us-isob-east-1:123456789012:runtime/r",
    "https://bedrock-agentcore.us-isob-east-1.sc2s.sgov.gov/runtimes/\
     arn%3Aaws-iso-b%3Abedrock-agentcore%3Aus-isob-east-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::iso_e(
    "arn:aws-iso-e:bedrock-agentcore:eu-isoe-west-1:123456789012:runtime/r",
    "https://bedrock-agentcore.eu-isoe-west-1.cloud.adc-e.uk/runtimes/\
     arn%3Aaws-iso-e%3Abedrock-agentcore%3Aeu-isoe-west-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::iso_f(
    "arn:aws-iso-f:bedrock-agentcore:us-isof-south-1:123456789012:runtime/r",
    "https://bedrock-agentcore.us-isof-south-1.csp.hci.ic.gov/runtimes/\
     arn%3Aaws-iso-f%3Abedrock-agentcore%3Aus-isof-south-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
#[case::iso(
    "arn:aws-iso:bedrock-agentcore:us-iso-east-1:123456789012:runtime/r",
    "https://bedrock-agentcore.us-iso-east-1.c2s.ic.gov/runtimes/\
     arn%3Aaws-iso%3Abedrock-agentcore%3Aus-iso-east-1%3A123456789012%3Aruntime%2Fr/invocations"
)]
fn strands_url_uses_the_partition_dns_suffix_of_the_arn_region(
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    let config = StrandsDeciderDecisionsConfig::default();
    assert_eq!(config.get_complete_url(api_base, "any-model"), expected);
}

#[test]
fn strands_url_percent_encodes_the_arn_in_the_path() {
    let config = StrandsDeciderDecisionsConfig::default();
    let url = config.get_complete_url(
        "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/a_b-c9",
        "any-model",
    );
    let path = url
        .strip_prefix("https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/")
        .expect("the runtime host");
    assert_eq!(
        path,
        "arn%3Aaws%3Abedrock-agentcore%3Aus-east-1%3A123456789012%3Aruntime%2Fa_b-c9/invocations"
    );
}

#[rstest]
#[case::plain("https://strands.example", "https://strands.example/v1/systemone")]
#[case::trailing_slash("https://strands.example/", "https://strands.example/v1/systemone")]
#[case::version_suffix("https://strands.example/v1", "https://strands.example/v1/systemone")]
#[case::version_and_slash("https://strands.example/v1/", "https://strands.example/v1/systemone")]
#[case::near_miss_arn(
    "arn:aws:bedrock-agentcore:us-east-1:12345678901:runtime/r",
    "arn:aws:bedrock-agentcore:us-east-1:12345678901:runtime/r/v1/systemone"
)]
fn strands_url_falls_back_to_the_parent_systemone_path_for_a_plain_base(
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    let config = StrandsDeciderDecisionsConfig::default();
    assert_eq!(config.get_complete_url(api_base, "any-model"), expected);
}
