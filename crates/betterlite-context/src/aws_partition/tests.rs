#![cfg(test)]

use super::*;

#[test]
fn call_context_tests_behavior() {
    assert_region_partitions();
    assert_bedrock_arns();
    assert_aws_arns();
}

fn assert_region_partitions() {
    assert_eq!(get_aws_partition(Some("cn-north-1")).partition, "aws-cn");
    assert_eq!(
        get_aws_partition(Some("us-gov-west-1")).partition,
        "aws-us-gov"
    );
    assert_eq!(
        get_aws_partition(Some("eu-isoe-1")).dns_suffix,
        "cloud.adc-e.uk"
    );
}

fn assert_bedrock_arns() {
    assert!(contains_bedrock_arn("x arn:aws-cn:bedrock:region:model/y"));
    assert!(contains_bedrock_arn(
        "arn:aws:bedrock-runtime:region:model/y"
    ));
    assert!(contains_bedrock_arn("arn:aws:bedrock-agent:region:model/y"));
    assert!(!contains_bedrock_arn("no arn here"));
    assert!(!is_bedrock_arn("x arn:aws:bedrock:region:model/y"));
    assert!(is_bedrock_arn("arn:aws-iso-b:bedrock:region:model/y"));
    assert!(!is_bedrock_arn("arn:aws-!:bedrock:region:model/y"));
    assert!(!is_bedrock_arn("arn:aws-:bedrock:region:model/y"));
}

fn assert_aws_arns() {
    assert!(contains_aws_arn("x arn:aws-cn:s3:region:x"));
    assert!(!contains_aws_arn("not an ARN"));
    assert!(contains_aws_arn("arn:aws-iso-b:s3:region:x"));
}
