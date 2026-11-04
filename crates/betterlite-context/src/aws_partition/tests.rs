#![cfg(test)]

use super::*;

#[test]
fn partition_and_arn_helpers_cover_known_partition_prefixes() {
    assert_eq!(get_aws_partition(Some("cn-north-1")).partition, "aws-cn");
    assert_eq!(
        get_aws_partition(Some("us-gov-west-1")).partition,
        "aws-us-gov"
    );
    assert_eq!(
        get_aws_partition(Some("eu-isoe-1")).dns_suffix,
        "cloud.adc-e.uk"
    );
    assert!(contains_bedrock_arn("x arn:aws-cn:bedrock:region:model/y"));
    assert!(!is_bedrock_arn("x arn:aws:bedrock:region:model/y"));
    assert!(contains_aws_arn("x arn:aws-cn:s3:region:x"));
}
