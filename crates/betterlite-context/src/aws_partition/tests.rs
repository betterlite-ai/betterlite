#![cfg(test)]

use super::{contains_aws_arn, contains_bedrock_arn, get_aws_partition, is_bedrock_arn};
use rstest::rstest;

#[rstest]
#[case::absent(None, "aws", "amazonaws.com")]
#[case::empty(Some(""), "aws", "amazonaws.com")]
#[case::commercial(Some("us-east-1"), "aws", "amazonaws.com")]
#[case::china(Some("cn-north-1"), "aws-cn", "amazonaws.com.cn")]
#[case::government(Some("us-gov-west-1"), "aws-us-gov", "amazonaws.com")]
#[case::iso_b(Some("us-isob-east-1"), "aws-iso-b", "sc2s.sgov.gov")]
#[case::iso_f(Some("us-isof-east-1"), "aws-iso-f", "csp.hci.ic.gov")]
#[case::iso(Some("us-iso-east-1"), "aws-iso", "c2s.ic.gov")]
#[case::iso_e(Some("eu-isoe-west-1"), "aws-iso-e", "cloud.adc-e.uk")]
#[case::case_sensitive(Some("CN-north-1"), "aws", "amazonaws.com")]
fn call_context_tests_behavior(
    #[case] region: Option<&str>,
    #[case] partition: &str,
    #[case] suffix: &str,
) {
    let actual = get_aws_partition(region);
    assert_eq!(actual.partition, partition);
    assert_eq!(actual.dns_suffix, suffix);
    assert_eq!(actual.arn_prefix, format!("arn:{partition}:"));
}

mod arns;
